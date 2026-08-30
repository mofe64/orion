"""Chatterbox Nano model adapter and Raspberry Pi benchmark."""

from __future__ import annotations

from dataclasses import asdict, dataclass
import json
from pathlib import Path
import resource
import time


ORION_PLAYBACK_SAMPLE_RATE = 48_000


class ChatterboxNanoSynthesizer:
    """Load Nano once and synthesize with its checkpoint's built-in voice."""

    def __init__(self, device: str = "cpu") -> None:
        import torchaudio
        from chatterbox.tts_turbo import ChatterboxTurboTTS

        self._torchaudio = torchaudio
        self._model = ChatterboxTurboTTS.from_pretrained(device=device, nano=True)

    def synthesize(self, text: str, output_path: Path) -> float:
        """Generate a 48 kHz stereo PCM WAV and return its duration."""
        wav = self._model.generate(text)
        if wav.ndim == 1:
            wav = wav.unsqueeze(0)
        if wav.shape[0] != 1:
            wav = wav[:1]
        if self._model.sr != ORION_PLAYBACK_SAMPLE_RATE:
            wav = self._torchaudio.functional.resample(
                wav, self._model.sr, ORION_PLAYBACK_SAMPLE_RATE
            )
        wav = wav.repeat(2, 1).clamp(-1.0, 1.0).cpu()
        output_path.parent.mkdir(parents=True, exist_ok=True)
        self._torchaudio.save(
            str(output_path),
            wav,
            ORION_PLAYBACK_SAMPLE_RATE,
            encoding="PCM_S",
            bits_per_sample=16,
        )
        return wav.shape[1] / ORION_PLAYBACK_SAMPLE_RATE


@dataclass(frozen=True)
class BenchmarkResult:
    model: str
    device: str
    load_seconds: float
    synthesis_seconds: list[float]
    audio_seconds: list[float]
    realtime_factors: list[float]
    peak_rss_kib: int


def benchmark(text: str, output_directory: Path, iterations: int) -> BenchmarkResult:
    if iterations <= 0:
        raise ValueError("benchmark iterations must be positive")

    load_started = time.monotonic()
    synthesizer = ChatterboxNanoSynthesizer(device="cpu")
    load_seconds = time.monotonic() - load_started

    synthesis_seconds: list[float] = []
    audio_seconds: list[float] = []
    realtime_factors: list[float] = []
    for index in range(iterations):
        started = time.monotonic()
        duration = synthesizer.synthesize(
            text, output_directory / f"chatterbox-nano-{index + 1}.wav"
        )
        elapsed = time.monotonic() - started
        synthesis_seconds.append(elapsed)
        audio_seconds.append(duration)
        realtime_factors.append(elapsed / duration)

    result = BenchmarkResult(
        model="ResembleAI/chatterbox-nano",
        device="cpu",
        load_seconds=load_seconds,
        synthesis_seconds=synthesis_seconds,
        audio_seconds=audio_seconds,
        realtime_factors=realtime_factors,
        peak_rss_kib=resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
    )
    print(json.dumps(asdict(result), separators=(",", ":")))
    return result
