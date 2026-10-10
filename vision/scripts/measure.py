#!/usr/bin/env python3
"""In-memory Pi 5 camera/inference benchmark. One JSON document on stdout.

This is commissioning tooling, not a vision service. Camera and inference
libraries are imported only during a run; --help works on a workstation.
"""

import argparse
import contextlib
import ctypes
import hashlib
import importlib.metadata
import json
import math
import os
from pathlib import Path
import platform
import statistics
import sys
import threading
import time


def distribution(values):
    """Summarise numeric measurements without inventing missing samples."""
    if not values:
        return {"samples": 0, "min": None, "median": None, "p95": None, "max": None}
    ordered = sorted(values)
    return {
        "samples": len(values),
        "min": ordered[0],
        "median": statistics.median(ordered),
        "p95": ordered[math.ceil(0.95 * len(ordered)) - 1],
        "max": ordered[-1],
    }


def cpu_temperature():
    try:
        value = float(Path("/sys/class/thermal/thermal_zone0/temp").read_text()) / 1000
        return value if math.isfinite(value) else None
    except (OSError, ValueError):
        return None


@contextlib.contextmanager
def library_diagnostics():
    """Redirect Python AND native library stdout while keeping JSON on fd 1."""
    # redirect_stdout alone misses printf/std::cout in OpenCV and NCNN. Flush
    # libc while redirected too, so buffered native logs cannot escape later.
    libc = ctypes.CDLL(None)
    libc.fflush.argtypes = [ctypes.c_void_p]
    libc.fflush.restype = ctypes.c_int
    sys.stdout.flush()
    libc.fflush(None)
    original = os.dup(1)
    try:
        os.dup2(2, 1)
        with contextlib.redirect_stdout(sys.stderr):
            yield
    finally:
        libc.fflush(None)
        os.dup2(original, 1)
        os.close(original)


def file_identity(path):
    # These are model files, never captured images.
    with path.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    return {"file": path.name, "sha256": digest}


class LatestFrame:
    """One copied lores buffer; callbacks replace it instead of queueing frames.

    main and lores buffers arrive in the same completed libcamera request.
    Count both streams there; copying main RGB pixels is unnecessary for an
    inference benchmark. Scalar statistics are collected only during a phase.
    """

    def __init__(self):
        self.condition = threading.Condition()
        self.latest = None
        self.sequence = 0
        self.last_sensor_ns = None
        self.phase = None
        self.error = None

    def capture(self, request):
        try:
            sensor_ns = request.get_metadata()["SensorTimestamp"]
            received_boot_ns = time.clock_gettime_ns(time.CLOCK_BOOTTIME)
            received_mono_ns = time.monotonic_ns()
            # make_array copies the camera buffer, so it remains valid after
            # Picamera2 releases the request. Never hold a camera request here.
            lores = request.make_array("lores")
            with self.condition:
                if not isinstance(sensor_ns, int) or sensor_ns <= 0:
                    raise ValueError("invalid SensorTimestamp")
                if self.last_sensor_ns is not None and sensor_ns <= self.last_sensor_ns:
                    raise ValueError("SensorTimestamp did not increase")
                self.last_sensor_ns = sensor_ns
                self.sequence += 1
                self.latest = (self.sequence, lores)
                if self.phase is not None:
                    phase = self.phase
                    phase["frames"] += 1
                    if phase["first_sensor_ns"] is None:
                        phase["first_sensor_ns"] = sensor_ns
                    phase["last_sensor_ns"] = sensor_ns
                    phase["delivery_age_ms"].append((received_boot_ns - sensor_ns) / 1e6)
                    phase["clock_offset_ns"].append(received_boot_ns - received_mono_ns)
                self.condition.notify_all()
        except Exception as error:
            # Report callback errors to the benchmark thread rather than
            # letting them silently kill Picamera2's event loop.
            with self.condition:
                self.error = error
                self.condition.notify_all()

    def next(self, after, timeout):
        with self.condition:
            self.condition.wait_for(
                lambda: self.error is not None or self.sequence > after, timeout=timeout
            )
            if self.error is not None:
                raise RuntimeError(f"camera callback failed: {self.error}") from self.error
            return self.latest if self.sequence > after else None

    def begin(self):
        with self.condition:
            self.phase = {
                "frames": 0, "first_sensor_ns": None, "last_sensor_ns": None,
                "delivery_age_ms": [], "clock_offset_ns": [],
            }
            # Start with a fresh frame, not the last phase's retained frame.
            return self.sequence

    def end(self):
        with self.condition:
            phase, self.phase = self.phase, None
            return phase


def lores_bgr(cv2, yuv):
    # PiSP may pad the 320px stream to a wider stride (384px on the lamp).
    # Convert the complete I420 planes first: cropping packed YUV columns would
    # corrupt U/V row packing. Then remove only the BGR padding columns.
    return cv2.cvtColor(yuv, cv2.COLOR_YUV2BGR_I420)[:, :320]


class YuNet:
    def __init__(self, cv2, path, threads):
        self.cv2 = cv2
        cv2.setNumThreads(threads)
        self.detector = cv2.FaceDetectorYN.create(
            str(path), "", (320, 240), 0.9, 0.3, 5000,
            cv2.dnn.DNN_BACKEND_OPENCV, cv2.dnn.DNN_TARGET_CPU,
        )

    def run(self, yuv):
        bgr = lores_bgr(self.cv2, yuv)
        self.detector.detect(bgr)


class YoloNCNN:
    """Forward pass of a local Ultralytics YOLO11n NCNN export at 320x320.

    NCNN thread count is set before loading. No Vulkan, torch, downloads, or
    postprocessing/NMS. Inference timing includes conversion and letterboxing.
    """

    def __init__(self, cv2, ncnn, param, weights, threads, input_name, output_name):
        self.cv2, self.ncnn = cv2, ncnn
        self.input_name, self.output_name = input_name, output_name
        cv2.setNumThreads(threads)
        self.net = ncnn.Net()
        self.net.opt.num_threads = threads
        self.net.opt.use_vulkan_compute = False
        if self.net.load_param(str(param)) != 0 or self.net.load_model(str(weights)) != 0:
            raise RuntimeError("could not load YOLO11n NCNN param/bin")

    def run(self, yuv):
        bgr = lores_bgr(self.cv2, yuv)
        # 320x240 lores -> 320x320 square without stretching the image.
        square = self.cv2.copyMakeBorder(
            bgr, 40, 40, 0, 0, self.cv2.BORDER_CONSTANT, value=(114, 114, 114)
        )
        tensor = self.ncnn.Mat.from_pixels(
            square, self.ncnn.Mat.PixelType.PIXEL_BGR2RGB, 320, 320
        )
        tensor.substract_mean_normalize([], [1 / 255] * 3)
        extractor = self.net.create_extractor()
        if extractor.input(self.input_name, tensor) != 0:
            raise RuntimeError(f"NCNN input {self.input_name!r} was rejected")
        status, _ = extractor.extract(self.output_name)
        if status != 0:
            raise RuntimeError(f"NCNN output {self.output_name!r} was rejected")


def run_phase(source, name, threads, detector, seconds, *, clock=time.monotonic,
              temperature=cpu_temperature):
    temperatures = []
    latencies = []
    used = 0
    after = source.begin()
    start = clock()
    deadline = start + seconds
    next_temperature = start
    try:
        while clock() < deadline:
            now = clock()
            if now >= next_temperature:
                value = temperature()
                if value is not None:
                    temperatures.append(value)
                next_temperature = now + 1
            frame = source.next(after, min(0.1, max(0, deadline - clock())))
            if frame is None:
                continue
            after, yuv = frame
            if detector is not None:
                started = clock()
                detector.run(yuv)
                latencies.append((clock() - started) * 1000)
                used += 1
    finally:
        counters = source.end()
    elapsed = clock() - start
    frames = counters["frames"]
    if frames < 2:
        raise RuntimeError(f"{name}/{threads}: fewer than two camera frames in measurement window")
    span = (counters["last_sensor_ns"] - counters["first_sensor_ns"]) / 1e9
    if detector is not None and not used:
        raise RuntimeError(f"{name}/{threads}: no inference completed")
    return {
        "name": name, "threads": threads, "requested_seconds": seconds,
        "elapsed_seconds": elapsed,
        "main_frames": frames, "lores_frames": frames,
        "main_fps": frames / elapsed, "lores_fps": frames / elapsed,
        "sensor_fps": (frames - 1) / span,
        "inferences": used,
        "inference_fps": used / elapsed if detector is not None else None,
        "frames_not_inferred": max(0, frames - used) if detector is not None else None,
        "inference_ms": distribution(latencies),
        "cpu_temperature_c": distribution(temperatures),
        "received_boottime_minus_sensor_ms": distribution(counters["delivery_age_ms"]),
        "boottime_minus_monotonic_ns": distribution(counters["clock_offset_ns"]),
        "first_sensor_ns": counters["first_sensor_ns"],
        "last_sensor_ns": counters["last_sensor_ns"],
    }


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--yunet", type=Path, required=True, help="local YuNet ONNX model")
    parser.add_argument("--yolo-param", type=Path, required=True, help="local YOLO11n NCNN .param")
    parser.add_argument("--yolo-bin", type=Path, required=True, help="local YOLO11n NCNN .bin")
    parser.add_argument("--yolo-input", default="in0")
    parser.add_argument("--yolo-output", default="out0")
    parser.add_argument("--duration", type=float, default=60, help="total timed seconds across five phases (5..600)")
    parser.add_argument("--label", choices=("idle", "voice"), default="idle")
    parser.add_argument("--camera", type=int, default=0, help="index from rpicam-hello --list-cameras")
    parser.add_argument("--hflip", action="store_true")
    parser.add_argument("--vflip", action="store_true")
    args = parser.parse_args(argv)
    if not math.isfinite(args.duration) or not 5 <= args.duration <= 600:
        parser.error("--duration must be between 5 and 600 seconds")
    if args.camera < 0:
        parser.error("--camera must be nonnegative")
    return args


def benchmark(args):
    models = {
        "yunet": file_identity(args.yunet),
        "yolo_param": file_identity(args.yolo_param),
        "yolo_bin": file_identity(args.yolo_bin),
    }
    # Keep unrelated numerical libraries from oversubscribing the four cores.
    # OpenCV and NCNN get their explicit one/two-thread settings per phase.
    for key in ("OPENBLAS_NUM_THREADS", "OMP_NUM_THREADS"):
        os.environ[key] = "1"
    import cv2
    import libcamera
    import ncnn
    from picamera2 import Picamera2

    collector = LatestFrame()
    camera = Picamera2(args.camera)
    phases = []
    try:
        config = camera.create_video_configuration(
            main={"size": (1280, 720), "format": "RGB888"},
            lores={"size": (320, 240), "format": "YUV420"},
            transform=libcamera.Transform(hflip=args.hflip, vflip=args.vflip),
            buffer_count=4, controls={"FrameRate": 30}, queue=False,
        )
        camera.configure(config)
        camera.post_callback = collector.capture
        camera.start(show_preview=False)
        if collector.next(0, 5) is None:
            raise RuntimeError("camera produced no frame within five seconds")
        # The first phase measures both streams without inference. The four
        # remaining phases share the same live camera configuration.
        phases.append(run_phase(collector, "capture_only", None, None, args.duration / 5))
        for name, factory in (
            ("yunet", lambda threads: YuNet(cv2, args.yunet, threads)),
            ("yolo11n_ncnn_320", lambda threads: YoloNCNN(
                cv2, ncnn, args.yolo_param, args.yolo_bin, threads,
                args.yolo_input, args.yolo_output)),
        ):
            for threads in (1, 2):
                detector = factory(threads)
                # Exclude one initial warmup from timing statistics.
                frame = collector.next(0, 1)
                if frame is None:
                    raise RuntimeError("no frame available for inference warmup")
                detector.run(frame[1])
                phases.append(run_phase(collector, name, threads, detector, args.duration / 5))
        camera_info = Picamera2.global_camera_info()[args.camera]
        camera_model = camera.camera_properties.get("Model")
        actual = camera.camera_configuration()
        streams = {name: {"size": list(actual[name]["size"]), "format": actual[name]["format"]}
                   for name in ("main", "lores")}
    finally:
        try:
            camera.stop()
        finally:
            camera.close()
    versions = {}
    for package in ("picamera2", "opencv-python-headless", "ncnn", "numpy"):
        try:
            versions[package] = importlib.metadata.version(package)
        except importlib.metadata.PackageNotFoundError:
            versions[package] = None
    return {
        "ok": True, "schema_version": 1, "label": args.label,
        "platform": {"machine": platform.machine(), "kernel": platform.release(), "python": platform.python_version()},
        "versions": versions, "models": models,
        "camera": {"index": args.camera, "id": camera_info.get("Id"), "model": camera_model,
                   "hflip": args.hflip, "vflip": args.vflip, "streams": streams},
        "timestamp": {"source": "SensorTimestamp", "unit": "ns", "clock": "CLOCK_BOOTTIME",
                      "host_clocks": ["CLOCK_BOOTTIME", "CLOCK_MONOTONIC"],
                      "event": "sensor metadata; exact exposure timing requires lamp validation"},
        "timing": "preprocessing plus inference; YuNet includes detection postprocessing; YOLO excludes NMS",
        "phases": phases,
        "qwen_asr_slowdown": None,
    }


def main(argv=None):
    args = parse_args(argv)
    try:
        with library_diagnostics():
            result = benchmark(args)
        status = 0
    except Exception as error:
        result = {"ok": False, "schema_version": 1, "error": f"{type(error).__name__}: {error}"}
        status = 1
    print(json.dumps(result, allow_nan=False))
    return status


if __name__ == "__main__":
    raise SystemExit(main())
