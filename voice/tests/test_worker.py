import json
from pathlib import Path
import socket
import tempfile
import threading
import time
import unittest

from orion_voice.protocol import SynthesisRequest
from orion_voice.worker import TtsWorker


class FakeSynthesizer:
    def synthesize(self, text: str, output_path: Path) -> float:
        output_path.write_bytes(b"RIFF\x04\x00\x00\x00WAVE")
        return 0.25


class FailingSynthesizer:
    def synthesize(self, text: str, output_path: Path) -> float:
        output_path.write_bytes(b"partial")
        raise RuntimeError("model failed")


class TtsWorkerTests(unittest.TestCase):
    def test_worker_returns_generated_wav_path(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            socket_path = root / "tts.sock"
            worker = TtsWorker(FakeSynthesizer(), socket_path, root / "output")
            thread = threading.Thread(target=worker.serve_forever)
            thread.start()
            try:
                for _ in range(100):
                    if socket_path.exists():
                        break
                    time.sleep(0.005)

                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
                    client.connect(str(socket_path))
                    client.sendall(SynthesisRequest(3, "Hello.").to_json_line())
                    response = json.loads(client.makefile("rb").readline())

                self.assertEqual(response["request_id"], 3)
                self.assertEqual(response["state"], "ready")
                self.assertTrue(Path(response["wav_path"]).is_file())
            finally:
                worker.stop()
                thread.join(timeout=1)
                worker.close()
            self.assertFalse(socket_path.exists())

    def test_worker_removes_partial_wav_after_failure(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            socket_path = root / "tts.sock"
            output_directory = root / "output"
            worker = TtsWorker(FailingSynthesizer(), socket_path, output_directory)
            thread = threading.Thread(target=worker.serve_forever)
            thread.start()
            try:
                for _ in range(100):
                    if socket_path.exists():
                        break
                    time.sleep(0.005)

                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
                    client.connect(str(socket_path))
                    client.sendall(SynthesisRequest(4, "Hello.").to_json_line())
                    response = json.loads(client.makefile("rb").readline())

                self.assertEqual(response["state"], "failed")
                self.assertIn("model failed", response["error"])
                self.assertFalse((output_directory / "speech-4.wav").exists())
            finally:
                worker.stop()
                thread.join(timeout=1)
                worker.close()


if __name__ == "__main__":
    unittest.main()
