from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class WakeDetection:
    name: str
    score: float


class RustpotterWakeDetector:
    provider = "rustpotter"

    def __init__(self, model: Path, threshold: float) -> None:
        if not model.is_file():
            raise RuntimeError(f"Rustpotter wake reference was not found at {model}.")
        try:
            from orion_rustpotter import Detector
        except ImportError as error:
            raise RuntimeError(
                "The Orion Rustpotter extension is not installed in the voice-worker environment."
            ) from error

        self.model_name = model.name
        self.threshold = threshold
        self._detector = Detector(str(model), threshold)

    def process(self, pcm: bytes) -> WakeDetection | None:
        detection = self._detector.process_pcm16(pcm)
        if detection is None:
            return None
        name, score = detection
        return WakeDetection(str(name), float(score))

    def reset(self) -> None:
        self._detector.reset()
