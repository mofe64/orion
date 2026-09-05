from __future__ import annotations

from array import array
from collections import deque
from dataclasses import dataclass
import math
import sys

FRAME_SAMPLES = 320
SAMPLE_RATE = 16_000


def pcm16_rms(pcm: bytes) -> float:
    """DC-corrected energy for endpoint decisions; never modifies capture audio."""
    if not pcm:
        return 0.0
    samples = array("h")
    samples.frombytes(pcm)
    if sys.byteorder != "little":
        samples.byteswap()
    mean = sum(samples) / len(samples)
    return math.sqrt(sum((sample - mean) ** 2 for sample in samples) / len(samples))


class ListeningNoise:
    """Bounded frame energies, excluding the latest second around the wake word."""

    def __init__(self) -> None:
        self._energies: deque[float] = deque(maxlen=300)  # Five seconds + one-second guard.

    def accept(self, pcm: bytes) -> None:
        if len(pcm) != FRAME_SAMPLES * 2:
            raise ValueError("Noise estimation requires one 20 ms PCM16 frame.")
        self._energies.append(pcm16_rms(pcm))

    def threshold(self) -> float:
        history = sorted(list(self._energies)[:-50])
        if len(history) < 25:
            return 500.0  # Not enough pre-wake history yet.
        # The lower quartile tolerates intermittent speech/noise in the listening window.
        noise = history[(len(history) - 1) // 4]
        return max(500.0, noise * 3.0)


@dataclass(frozen=True)
class EndpointConfig:
    speech_rms: float = 500.0
    min_speech_ms: int = 240
    min_capture_ms: int = 1_200
    trailing_silence_ms: int = 1_000
    speech_confirmation_ms: int = 60
    max_utterance_ms: int = 15_000


class EnergyEndpointDetector:
    """Small deterministic endpoint detector; it decides when ASR should run."""

    def __init__(self, config: EndpointConfig = EndpointConfig()) -> None:
        self.config = config
        self._pending = bytearray()
        self._speech_ms = 0
        self._silence_ms = 0
        self._total_ms = 0
        self._speech_run_ms = 0
        self.end_reason: str | None = None

    @property
    def capture_ms(self) -> int:
        return self._total_ms

    def accept(self, pcm: bytes) -> bool:
        if self.end_reason is not None:
            return True
        self._pending.extend(pcm)
        frame_bytes = FRAME_SAMPLES * 2
        frame_ms = round(FRAME_SAMPLES * 1_000 / SAMPLE_RATE)
        while len(self._pending) >= frame_bytes:
            frame = bytes(self._pending[:frame_bytes])
            del self._pending[:frame_bytes]
            self._total_ms += frame_ms
            self._silence_ms += frame_ms
            if pcm16_rms(frame) >= self.config.speech_rms:
                self._speech_run_ms += frame_ms
                if self._speech_run_ms >= self.config.speech_confirmation_ms:
                    self._speech_ms += (self._speech_run_ms
                                        if self._speech_run_ms == self.config.speech_confirmation_ms
                                        else frame_ms)
                    self._silence_ms = 0
            else:
                self._speech_run_ms = 0

            enough_speech = self._speech_ms >= self.config.min_speech_ms
            past_grace = self._total_ms >= self.config.min_capture_ms
            if (past_grace and enough_speech and self._speech_run_ms == 0
                    and self._silence_ms >= self.config.trailing_silence_ms):
                self.end_reason = "silence"
                return True
            if self._total_ms >= self.config.max_utterance_ms:
                self.end_reason = "max_duration"
                return True
        return False

    def prime_detected_speech(self) -> None:
        self._speech_ms = self.config.min_speech_ms
        self._silence_ms = 0
