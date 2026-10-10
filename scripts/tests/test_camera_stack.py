"""Camera commissioning tooling without camera hardware or model packages."""

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock, patch


ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / "vision/scripts"
spec = importlib.util.spec_from_file_location("camera_measure", SCRIPTS / "measure.py")
measure = importlib.util.module_from_spec(spec)
spec.loader.exec_module(measure)


class MeasurementTests(unittest.TestCase):
    def request(self, sensor, frame):
        return SimpleNamespace(get_metadata=lambda: {"SensorTimestamp": sensor},
                               make_array=lambda stream: frame)

    def test_latest_frame_replaces_backlog_and_rejects_duplicate_timestamps(self):
        source = measure.LatestFrame()
        with patch.object(measure.time, "CLOCK_BOOTTIME", 7, create=True), \
                patch.object(measure.time, "clock_gettime_ns", return_value=200, create=True):
            source.capture(self.request(100, "first"))
            source.capture(self.request(101, "latest"))
            self.assertEqual(source.next(0, 0), (2, "latest"))
            self.assertIsNone(source.next(2, 0))
            source.capture(self.request(101, "duplicate"))
            with self.assertRaisesRegex(RuntimeError, "did not increase"):
                source.next(2, 0)

    def test_missing_sensor_timestamp_is_an_error_not_a_host_time_fallback(self):
        source = measure.LatestFrame()
        source.capture(SimpleNamespace(get_metadata=lambda: {}, make_array=lambda _: None))
        with self.assertRaisesRegex(RuntimeError, "SensorTimestamp"):
            source.next(0, 0)

    def test_phase_reports_delivery_separately_from_slow_inference(self):
        class Clock:
            now = 0
            def __call__(self):
                return self.now
        clock = Clock()
        source = measure.LatestFrame()
        def advance(seconds):
            clock.now += seconds
            with patch.object(measure.time, "CLOCK_BOOTTIME", 7, create=True), \
                    patch.object(measure.time, "clock_gettime_ns", return_value=int((clock.now + 1) * 1e9), create=True), \
                    patch.object(measure.time, "monotonic_ns", return_value=int(clock.now * 1e9)):
                source.capture(self.request(int(clock.now * 1e9), "yuv"))
        original_next = source.next
        def next_frame(after, timeout):
            advance(0.04)
            return original_next(after, 0)
        source.next = next_frame
        def infer(frame):
            self.assertEqual(frame, "yuv")
            for _ in range(4):
                advance(0.04)
        result = measure.run_phase(source, "slow", 1, SimpleNamespace(run=infer),
                                   1, clock=clock, temperature=lambda: 55)
        self.assertEqual(result["main_frames"], result["lores_frames"])
        self.assertEqual(result["main_frames"], 25)
        self.assertEqual(result["inferences"], 5)
        self.assertEqual(result["frames_not_inferred"], 20)
        self.assertAlmostEqual(result["inference_ms"]["median"], 160)
        self.assertAlmostEqual(result["sensor_fps"], 25)
        self.assertEqual(result["cpu_temperature_c"]["median"], 55)
        self.assertEqual(result["boottime_minus_monotonic_ns"]["median"], 1_000_000_000)
        self.assertIsNone(source.phase)

    def test_stalled_camera_finishes_window_and_reports_failure(self):
        source = measure.LatestFrame()
        with self.assertRaisesRegex(RuntimeError, "fewer than two"):
            measure.run_phase(source, "capture_only", None, None, 0.02)
        self.assertIsNone(source.phase)

    def test_json_stdout_and_nonzero_status_on_failure(self):
        args = ["--yunet", "missing.onnx", "--yolo-param", "missing.param", "--yolo-bin", "missing.bin"]
        output, diagnostics = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(diagnostics):
            status = measure.main(args)
        self.assertEqual(status, 1)
        result = json.loads(output.getvalue())
        self.assertFalse(result["ok"])
        self.assertIn("FileNotFoundError", result["error"])

    def test_json_success_redirects_library_prints(self):
        def fake_benchmark(args):
            print("library log")
            return {"ok": True}
        with patch.object(measure, "benchmark", side_effect=fake_benchmark):
            output, diagnostics = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(output), contextlib.redirect_stderr(diagnostics):
                status = measure.main(["--yunet", "x", "--yolo-param", "y", "--yolo-bin", "z"])
        self.assertEqual(status, 0)
        self.assertEqual(json.loads(output.getvalue()), {"ok": True})
        self.assertEqual(diagnostics.getvalue(), "library log\n")

    def test_native_stdout_diagnostics_cannot_pollute_json(self):
        code = '''
import ctypes, importlib.util, os, sys
spec = importlib.util.spec_from_file_location("measure", sys.argv[1])
measure = importlib.util.module_from_spec(spec)
spec.loader.exec_module(measure)
def benchmark(args):
    print("Python diagnostic")
    os.write(1, b"native unbuffered diagnostic\\n")
    libc = ctypes.CDLL(None)
    libc.printf(b"native buffered diagnostic\\n")
    return {"ok": True}
measure.benchmark = benchmark
sys.exit(measure.main(["--yunet", "x", "--yolo-param", "y", "--yolo-bin", "z"]))
'''
        result = subprocess.run([sys.executable, "-c", code, str(SCRIPTS / "measure.py")],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout), {"ok": True})
        for diagnostic in ("Python diagnostic", "native unbuffered diagnostic", "native buffered diagnostic"):
            self.assertIn(diagnostic, result.stderr)

    def test_invalid_duration_cannot_create_an_unbounded_run(self):
        for duration in ("nan", "inf", "0", "601"):
            with self.subTest(duration=duration), contextlib.redirect_stderr(io.StringIO()), \
                    self.assertRaises(SystemExit):
                measure.parse_args(["--yunet", "x", "--yolo-param", "y", "--yolo-bin", "z",
                                    "--duration", duration])

    def test_benchmark_configures_both_streams_all_thread_counts_and_closes_camera(self):
        with tempfile.TemporaryDirectory() as directory:
            paths = [Path(directory) / name for name in ("face.onnx", "model.param", "model.bin")]
            for path in paths:
                path.write_bytes(b"test model identity")
            args = measure.parse_args(["--yunet", str(paths[0]), "--yolo-param", str(paths[1]),
                                       "--yolo-bin", str(paths[2]), "--duration", "5", "--vflip"])
            cv2 = MagicMock()
            net = MagicMock()
            net.load_param.return_value = net.load_model.return_value = 0
            net.create_extractor.return_value.input.return_value = 0
            net.create_extractor.return_value.extract.return_value = (0, "result")
            ncnn = MagicMock()
            ncnn.Net.return_value = net
            camera = MagicMock()
            camera.camera_properties = {"Model": "imx708_wide"}
            config = {"main": {"size": (1280, 720), "format": "RGB888"},
                      "lores": {"size": (320, 240), "format": "YUV420"}}
            camera.camera_configuration.return_value = config
            camera.start.side_effect = lambda **_: camera.post_callback(self.request(1, "yuv"))
            picamera = MagicMock(return_value=camera)
            picamera.global_camera_info.return_value = [{"Id": "test/cam0"}]
            libcamera = MagicMock()
            modules = {"cv2": cv2, "ncnn": ncnn, "libcamera": libcamera,
                       "picamera2": SimpleNamespace(Picamera2=picamera)}
            settings = []
            def fake_phase(source, name, threads, detector, seconds):
                settings.append((name, threads, seconds))
                if detector is not None and isinstance(detector, measure.YoloNCNN):
                    self.assertEqual(detector.net.opt.num_threads, threads)
                    self.assertFalse(detector.net.opt.use_vulkan_compute)
                return {"name": name}
            with patch.dict(sys.modules, modules), patch.dict(os.environ), \
                    patch.object(measure.time, "CLOCK_BOOTTIME", 7, create=True), \
                    patch.object(measure.time, "clock_gettime_ns", return_value=100, create=True), \
                    patch.object(measure, "run_phase", side_effect=fake_phase):
                result = measure.benchmark(args)
            self.assertTrue(result["ok"])
            self.assertEqual(settings, [("capture_only", None, 1), ("yunet", 1, 1), ("yunet", 2, 1),
                                        ("yolo11n_ncnn_320", 1, 1), ("yolo11n_ncnn_320", 2, 1)])
            self.assertEqual(camera.create_video_configuration.call_args.kwargs["main"], config["main"])
            self.assertEqual(camera.create_video_configuration.call_args.kwargs["lores"], config["lores"])
            libcamera.Transform.assert_called_once_with(hflip=False, vflip=True)
            self.assertEqual(cv2.setNumThreads.call_args_list,
                             [unittest.mock.call(1), unittest.mock.call(2)] * 2)
            self.assertEqual(result["timestamp"]["clock"], "CLOCK_BOOTTIME")
            self.assertEqual(result["camera"]["id"], "test/cam0")
            self.assertEqual(len(result["models"]["yunet"]["sha256"]), 64)
            camera.stop.assert_called_once()
            camera.close.assert_called_once()
            # A failing capture phase must still close the camera.
            camera.reset_mock()
            with patch.dict(sys.modules, modules), patch.dict(os.environ), \
                    patch.object(measure.time, "CLOCK_BOOTTIME", 7, create=True), \
                    patch.object(measure.time, "clock_gettime_ns", return_value=100, create=True), \
                    patch.object(measure, "run_phase", side_effect=RuntimeError("capture failed")), \
                    self.assertRaisesRegex(RuntimeError, "capture failed"):
                measure.benchmark(args)
            camera.stop.assert_called_once()
            camera.close.assert_called_once()

    def test_yolo_letterbox_normalisation_and_rejected_output(self):
        cv2, ncnn = MagicMock(), MagicMock()
        net = ncnn.Net.return_value
        net.load_param.return_value = net.load_model.return_value = 0
        net.create_extractor.return_value.input.return_value = 0
        net.create_extractor.return_value.extract.return_value = (0, "result")
        detector = measure.YoloNCNN(cv2, ncnn, Path("model.param"), Path("model.bin"), 2, "in0", "out0")
        detector.run("yuv")
        cv2.copyMakeBorder.assert_called_once_with(cv2.cvtColor.return_value, 40, 40, 0, 0,
                                                   cv2.BORDER_CONSTANT, value=(114, 114, 114))
        ncnn.Mat.from_pixels.assert_called_once_with(cv2.copyMakeBorder.return_value,
                                                    ncnn.Mat.PixelType.PIXEL_BGR2RGB, 320, 320)
        ncnn.Mat.from_pixels.return_value.substract_mean_normalize.assert_called_once_with([], [1 / 255] * 3)
        net.create_extractor.return_value.extract.return_value = (1, None)
        with self.assertRaisesRegex(RuntimeError, "NCNN output"):
            detector.run("yuv")


class StackScriptTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.prefix = self.root / "local"
        self.modules = self.root / "modules"
        for module in ("rp1_cfe", "pisp_be", "imx708"):
            (self.modules / module).mkdir(parents=True)
        self.log = self.root / "calls"
        self.environment = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}",
                                TEST_LOG=str(self.log), TEST_PREFIX=str(self.prefix))

    def tool(self, name, body):
        path = self.bin / name
        path.write_text('#!/bin/bash\nset -euo pipefail\nprintf "%s\\n" "$(basename "$0") $*" >> "$TEST_LOG"\n' + body)
        path.chmod(0o755)
        return path

    def verifier(self, camera="imx708_wide", import_ok=True):
        hello = self.tool("rpicam-hello", f"echo '{camera}'\n")
        self.tool("python3", "exit 0\n" if import_ok else "exit 1\n")
        self.tool("timeout", 'shift; exec "$@"\n')
        source = (SCRIPTS / "verify-camera-stack.sh").read_text()
        source = source.replace("/usr/local/bin/rpicam-hello", str(hello))
        source = source.replace("/sys/module", str(self.modules))
        path = self.root / "verify.sh"
        path.write_text(source)
        return subprocess.run(["bash", str(path)], env=self.environment, capture_output=True, text=True)

    def test_verify_requires_sensor_bindings_and_all_three_drivers(self):
        self.assertEqual(self.verifier().returncode, 0)
        self.assertNotEqual(self.verifier(camera="imx219").returncode, 0)
        self.assertNotEqual(self.verifier(import_ok=False).returncode, 0)
        (self.modules / "pisp_be").rmdir()
        result = self.verifier()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("pisp_be is not loaded", result.stderr)

    def installer(self, wrong_host=False, moved_tag=False, no_conflicts=False):
        release = self.root / "os-release"
        release.write_text("ID=ubuntu\nVERSION_ID=24.04\n")
        model = self.root / "model"
        model.write_text("Raspberry Pi 4" if wrong_host else "Raspberry Pi 5")
        self.tool("uname", "echo aarch64\n")
        for name in ("apt-get", "ldconfig", "udevadm", "getent", "usermod", "id"):
            self.tool(name, "exit 0\n")
        self.tool("dpkg-query", "exit 1\n" if no_conflicts else
                  'printf "libcamera0.2:arm64\\tii \\npython3-libcamera\\tii \\nlibcamera-dev\\trc \\n"\n')
        self.tool("git", '''
if [[ $1 == clone ]]; then
    mkdir -p "${@: -1}"
elif [[ $1 == -C ]]; then
    case "$2" in
      */libpisp) echo 50426319aa1a9ba4672f91977429365ad4e335a2 ;;
      */libcamera) echo 29156679717bec7cc4784aeba3548807f2c27fca ;;
      */rpicam-apps) echo 5a3f5965aca96c2d575261e4e4045f0e0481279b ;;
    esac
fi
'''.replace("echo 50426319aa1a9ba4672f91977429365ad4e335a2", "echo moved" if moved_tag else "echo 50426319aa1a9ba4672f91977429365ad4e335a2"))
        self.tool("meson", '''
if [[ $1 == install ]]; then
    mkdir -p "$TEST_PREFIX/lib/python3.12/site-packages/libcamera"
    touch "$TEST_PREFIX/lib/python3.12/site-packages/libcamera/__init__.py"
fi
''')
        python = self.tool("python3", 'if [[ $* == *sys.version_info* ]]; then echo 3.12; fi\n')
        source = (SCRIPTS / "install-camera-stack.sh").read_text()
        source = source.replace("if (( EUID != 0 )); then", "if false; then")
        source = source.replace("/etc/os-release", str(release)).replace("/proc/device-tree/model", str(model))
        source = source.replace("export PATH=/usr/bin:/bin:/usr/local/bin", "# Retain the mock PATH")
        source = source.replace("/usr/bin/python3", str(python)).replace("/usr/local", str(self.prefix))
        source = source.replace("/etc/udev/rules.d", str(self.root / "rules"))
        source = source.replace("/var/tmp/orion-camera.XXXXXXXX", str(self.root / "build.XXXXXXXX"))
        path = self.root / "install.sh"
        path.write_text(source)
        return subprocess.run(["bash", str(path), "camera-user"], env=self.environment,
                              capture_output=True, text=True)

    def test_install_is_repeatable_and_purges_only_installed_camera_packages(self):
        first = self.installer()
        self.assertEqual(first.returncode, 0, first.stderr)
        rule = self.root / "rules/70-orion-camera.rules"
        pth = self.prefix / "lib/python3.12/dist-packages/orion-libcamera.pth"
        initial = rule.read_text(), pth.read_text()
        second = self.installer(no_conflicts=True)
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertEqual(initial, (rule.read_text(), pth.read_text()))
        log = self.log.read_text()
        self.assertEqual(log.count("apt-get purge -y libcamera0.2:arm64 python3-libcamera\n"), 1)
        self.assertNotIn("autoremove", log)
        self.assertIn("-Dpipelines=rpi/pisp -Dipas=rpi/pisp -Dpycamera=enabled", log)
        self.assertIn("-Denable_qt=disabled", log)
        self.assertEqual(log.count("usermod -a -G video camera-user"), 2)
        self.assertFalse(list(self.root.glob("build.*")))

    def test_wrong_platform_stops_before_package_changes(self):
        result = self.installer(wrong_host=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Requires Ubuntu", result.stderr)
        self.assertNotIn("apt-get", self.log.read_text())

    def test_moved_release_tag_is_rejected_before_build(self):
        result = self.installer(moved_tag=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Pinned commit mismatch", result.stderr)
        self.assertNotIn("meson setup", self.log.read_text())


if __name__ == "__main__":
    unittest.main()
