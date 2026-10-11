"""Fixed-audio ASR timing without a live model, camera, or service."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import MagicMock, patch
import wave


ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("asr_timing", ROOT / "vision/scripts/asr_timing.py")
timing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(timing)


class AsrTimingTests(unittest.TestCase):
    def wav(self, path, *, channels=1, width=2, rate=16000, frames=160):
        with wave.open(str(path), "wb") as audio:
            audio.setparams((channels, width, rate, 0, "NONE", ""))
            audio.writeframes(bytes(frames * channels * width))

    def test_fixture_is_fixed_pcm_with_digest_and_duration(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.wav"
            self.wav(path)
            pcm, metadata = timing.read_fixture(path)
            self.assertEqual(pcm, bytes(320))
            self.assertEqual(metadata["duration_ms"], 10)
            self.assertEqual(len(metadata["sha256"]), 64)
            self.assertEqual((pcm, metadata), timing.read_fixture(path))

    def test_wrong_format_empty_long_and_truncated_audio_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.wav"
            for options in ({"channels": 2}, {"width": 1}, {"rate": 8000},
                            {"frames": 0}, {"frames": 33 * 16000 + 1}):
                with self.subTest(options=options):
                    self.wav(path, **options)
                    with self.assertRaises(ValueError):
                        timing.read_fixture(path)
            self.wav(path)
            path.write_bytes(path.read_bytes()[:-2])
            with self.assertRaisesRegex(ValueError, "truncated"):
                timing.read_fixture(path)

    def test_one_warmup_then_n_identical_runs_and_nearest_rank_p95(self):
        transcriber = MagicMock()
        factory = MagicMock(return_value=transcriber)
        clock = iter([0, 0.1, 1, 1.2, 2, 2.3, 3, 3.4])
        result = timing.benchmark(b"fixed pcm", "model", Path("/private-server"), 4,
                                  factory=factory, clock=lambda: next(clock))
        factory.assert_called_once_with("model")
        self.assertEqual(transcriber.transcribe.call_count, 5)
        for call in transcriber.transcribe.call_args_list:
            self.assertEqual(call.args, (b"fixed pcm",))
        self.assertAlmostEqual(result["median_ms"], 250)
        self.assertAlmostEqual(result["p95_ms"], 400)
        self.assertEqual(result["timed_runs"], 4)
        transcriber.close.assert_called_once_with()

    def test_error_and_interrupt_close_only_the_owned_adapter_and_restore_environment(self):
        for error in (RuntimeError("warmup failed"), KeyboardInterrupt()):
            with self.subTest(error=error), patch.dict(os.environ, {
                    "ORION_LLAMA_SERVER": "/live-server", "PYTHONPATH": "/previous",
                    "ORION_ASR_CAPTURE_DIR": "/must-not-write"}):
                before = dict(os.environ)
                transcriber = MagicMock()
                transcriber.transcribe.side_effect = error
                def factory(model):
                    self.assertEqual(os.environ["ORION_LLAMA_SERVER"], "/private-server")
                    self.assertTrue(os.environ["PYTHONPATH"].startswith(str(timing.SPEECH_ROOT)))
                    self.assertNotIn("ORION_ASR_CAPTURE_DIR", os.environ)
                    return transcriber
                with self.assertRaises(type(error)):
                    timing.benchmark(b"pcm", "model", Path("/private-server"), 1, factory=factory)
                transcriber.close.assert_called_once_with()
                self.assertEqual(dict(os.environ), before)

    def test_timed_run_failure_still_closes_the_owned_server(self):
        transcriber = MagicMock()
        transcriber.transcribe.side_effect = [None, RuntimeError("timed run failed")]
        with self.assertRaisesRegex(RuntimeError, "timed run failed"):
            timing.benchmark(b"pcm", "model", Path("/server"), 2,
                             factory=lambda _: transcriber)
        transcriber.close.assert_called_once_with()

    def test_production_adapter_launches_private_server_with_production_flags(self):
        process = MagicMock()
        process.poll.return_value = None
        response = {"choices": [{"finish_reason": "stop", "message": {"content": "fixture"}}]}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            for name in ("model.gguf", "mmproj.gguf", "llama-server"):
                (root / name).touch()
            with patch.dict(os.environ, {"ORION_ASR_THREADS": "3"}), \
                    patch("orion_speech_worker.pi.subprocess.Popen", return_value=process) as launch, \
                    patch.object(timing.QwenGgufTranscriber, "request", return_value=response) as request:
                timing.benchmark(bytes(320), root, root / "llama-server", 1)
        command = launch.call_args.args[0]
        self.assertEqual(command[:3], [timing.sys.executable, "-m", "orion_speech_worker.native"])
        self.assertEqual(command[command.index("--host") + 1], "127.0.0.1")
        self.assertGreater(int(command[command.index("--port") + 1]), 0)
        self.assertEqual(len(command[command.index("--api-key") + 1]), 64)
        self.assertEqual(command[command.index("-t"):], [
            "-t", "3", "-tb", "3", "-ngl", "0", "-c", "4096", "-np", "1",
            "--no-warmup", "--log-disable"])
        self.assertEqual([call.args[0] for call in request.call_args_list],
                         ["/health", "/v1/chat/completions", "/v1/chat/completions"])
        process.terminate.assert_called_once_with()
        process.wait.assert_called_once_with(timeout=5)

    def test_invalid_run_count_does_not_start_server(self):
        factory = MagicMock()
        for count in (0, -1, 1001):
            with self.assertRaisesRegex(ValueError, "runs"):
                timing.benchmark(b"pcm", "model", Path("/server"), count, factory=factory)
        factory.assert_not_called()

    def test_cli_failure_is_json_and_nonzero(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code = timing.main(["--wav", "/no-such-fixture.wav"])
        self.assertEqual(code, 1)
        self.assertFalse(json.loads(output.getvalue())["ok"])

    def test_cli_success_is_one_json_record_without_transcripts(self):
        output = io.StringIO()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.wav"
            self.wav(path)
            with patch.object(timing.sys, "platform", "linux"), \
                    patch.object(timing, "benchmark", return_value={"median_ms": 2, "p95_ms": 3}) as run, \
                    contextlib.redirect_stdout(output):
                code = timing.main(["--wav", str(path), "--model", directory,
                                    "--llama-server", str(path), "--label", "tracking-off"])
        result = json.loads(output.getvalue())
        self.assertEqual(code, 0)
        self.assertTrue(result["ok"])
        self.assertEqual(result["label"], "tracking-off")
        self.assertNotIn("transcript", result)
        self.assertEqual(run.call_args.args[-1], 10)


if __name__ == "__main__":
    unittest.main()
