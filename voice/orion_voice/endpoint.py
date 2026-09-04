from __future__ import annotations

from array import array
from dataclasses import dataclass
import math
import sys

FRAME_SAMPLES = 320
SAMPLE_RATE = 16_000


def pcm16_rms(pcm: bytes) -> float:
    if not pcm:
        return 0.0
    samples = array("h")
    samples.frombytes(pcm)
    if sys.byteorder != "little":
        samples.byteswap()
    return math.sqrt(sum(sample * sample for sample in samples) / len(samples))


@dataclass(frozen=True)
class EndpointConfig:
    speech_rms: float = 500.0
    min_speech_ms: int = 240
    min_capture_ms: int = 1_200
    trailing_silence_ms: int = 800
    max_utterance_ms: int = 15_000


class EnergyEndpointDetector:
    """Small deterministic endpoint detector; it decides when ASR should run."""

    def __init__(self, config: EndpointConfig = EndpointConfig()) -> None:
        self.config = config
        self._pending = bytearray()
        self._speech_ms = 0
        self._silence_ms = 0
        self._total_ms = 0

    def accept(self, pcm: bytes) -> bool:
        self._pending.extend(pcm)
        frame_bytes = FRAME_SAMPLES * 2
        frame_ms = round(FRAME_SAMPLES * 1_000 / SAMPLE_RATE)
        while len(self._pending) >= frame_bytes:
            frame = bytes(self._pending[:frame_bytes])
            del self._pending[:frame_bytes]
            self._total_ms += frame_ms
            if pcm16_rms(frame) >= self.config.speech_rms:
                self._speech_ms += frame_ms
                self._silence_ms = 0
            elif self._speech_ms > 0:
                self._silence_ms += frame_ms

            enough_speech = self._speech_ms >= self.config.min_speech_ms
            past_grace = self._total_ms >= self.config.min_capture_ms
            if past_grace and enough_speech and self._silence_ms >= self.config.trailing_silence_ms:
                return True
            if self._total_ms >= self.config.max_utterance_ms:
                return True
        return False

    def prime_detected_speech(self) -> None:
        self._speech_ms = self.config.min_speech_ms
        self._silence_ms = 0
