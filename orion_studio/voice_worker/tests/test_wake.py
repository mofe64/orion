import sys
import tempfile
from pathlib import Path
import types
import unittest
from unittest.mock import patch

from orion_voice_worker.wake import RustpotterWakeDetector


class FakeNativeDetector:
    def __init__(self, model: str, threshold: float) -> None:
        self.model = model
        self.threshold = threshold
        self.reset_count = 0

    def process_pcm16(self, pcm: bytes):
        return ("hey orion", 0.61) if pcm else None

    def reset(self) -> None:
        self.reset_count += 1


class RustpotterWakeDetectorTests(unittest.TestCase):
    def test_adapts_native_detection_to_worker_contract(self) -> None:
        module = types.SimpleNamespace(Detector=FakeNativeDetector)
        with tempfile.TemporaryDirectory() as directory:
            model = Path(directory) / "hey_orion.rpw"
            model.touch()
            with patch.dict(sys.modules, {"orion_rustpotter": module}):
                detector = RustpotterWakeDetector(model, 0.4)
                detection = detector.process(b"\x00\x00")

        self.assertEqual(detection.name, "hey orion")
        self.assertAlmostEqual(detection.score, 0.61)
        self.assertEqual(detector.threshold, 0.4)

    def test_requires_a_commissioned_model_file(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "not found"):
            RustpotterWakeDetector(Path("missing.rpw"), 0.4)


if __name__ == "__main__":
    unittest.main()
