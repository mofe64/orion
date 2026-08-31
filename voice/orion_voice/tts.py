"""Piper model adapter and Raspberry Pi benchmark."""

from __future__ import annotations

import audioop
from dataclasses import asdict, dataclass
import json
from pathlib import Path
import resource
import time
import wave


ORION_PLAYBACK_SAMPLE_RATE = 48_000
DEFAULT_PIPER_VOICE_NAME = "en_US-lessac-medium"
DEFAULT_PIPER_MODEL_DIRECTORY = Path(__file__).resolve().parents[1] / "models"
DEFAULT_PIPER_MODEL_PATH = (
    DEFAULT_PIPER_MODEL_DIRECTORY / f"{DEFAULT_PIPER_VOICE_NAME}.onnx"
)


class PiperSynthesizer:
    """Load one Piper voice and produce Orion's physical playback format."""

    def __init__(self, model_path: Path, voice_loader=None) -> None:
        self.model_path = Path(model_path)
        config_path = Path(f"{self.model_path}.json")
        if not self.model_path.is_file():
            raise FileNotFoundError(
                f"Piper voice model not found: {self.model_path}. "
                "Run `python -m piper.download_voices --download-dir voice/models "
                f"{DEFAULT_PIPER_VOICE_NAME}` from the Orion repository."
            )
        if not config_path.is_file():
            raise FileNotFoundError(f"Piper voice config not found: {config_path}")

        if voice_loader is None:
            from piper import PiperVoice

            voice_loader = PiperVoice.load
        self._voice = voice_loader(
            self.model_path,
            config_path=config_path,
            use_cuda=False,
        )

    def synthesize(self, text: str, output_path: Path) -> float:
        """Generate a 48 kHz stereo PCM WAV and return its duration."""
        output_path.parent.mkdir(parents=True, exist_ok=True)
        frames_written = 0
        rate_state = None
        source_rate: int | None = None
        try:
            with wave.open(str(output_path), "wb") as output:
                output.setnchannels(2)
                output.setsampwidth(2)
                output.setframerate(ORION_PLAYBACK_SAMPLE_RATE)

                for chunk in self._voice.synthesize(text):
                    if chunk.sample_width != 2 or chunk.sample_channels != 1:
                        raise RuntimeError(
                            "Piper returned an unsupported audio format; expected "
                            "16-bit mono PCM"
                        )
                    if source_rate is None:
                        source_rate = chunk.sample_rate
                    elif chunk.sample_rate != source_rate:
                        raise RuntimeError("Piper changed sample rate during synthesis")

                    resampled, rate_state = audioop.ratecv(
                        chunk.audio_int16_bytes,
                        chunk.sample_width,
                        chunk.sample_channels,
                        chunk.sample_rate,
                        ORION_PLAYBACK_SAMPLE_RATE,
                        rate_state,
                    )
                    stereo = audioop.tostereo(resampled, 2, 1.0, 1.0)
                    output.writeframes(stereo)
                    frames_written += len(stereo) // 4
        except Exception:
            output_path.unlink(missing_ok=True)
            raise

        if frames_written == 0:
            output_path.unlink(missing_ok=True)
            raise RuntimeError("Piper generated no audio")
        return frames_written / ORION_PLAYBACK_SAMPLE_RATE


@dataclass(frozen=True)
class BenchmarkResult:
    model: str
    device: str
    load_seconds: float
    synthesis_seconds: list[float]
    audio_seconds: list[float]
    realtime_factors: list[float]
    peak_rss_kib: int


def benchmark(
    text: str,
    output_directory: Path,
    iterations: int,
    model_path: Path = DEFAULT_PIPER_MODEL_PATH,
) -> BenchmarkResult:
    if iterations <= 0:
        raise ValueError("benchmark iterations must be positive")

    load_started = time.monotonic()
    synthesizer = PiperSynthesizer(model_path)
    load_seconds = time.monotonic() - load_started

    synthesis_seconds: list[float] = []
    audio_seconds: list[float] = []
    realtime_factors: list[float] = []
    for index in range(iterations):
        started = time.monotonic()
        duration = synthesizer.synthesize(
            text, output_directory / f"piper-{index + 1}.wav"
        )
        elapsed = time.monotonic() - started
        synthesis_seconds.append(elapsed)
        audio_seconds.append(duration)
        realtime_factors.append(elapsed / duration)

    result = BenchmarkResult(
        model=model_path.stem,
        device="cpu",
        load_seconds=load_seconds,
        synthesis_seconds=synthesis_seconds,
        audio_seconds=audio_seconds,
        realtime_factors=realtime_factors,
        peak_rss_kib=resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
    )
    print(json.dumps(asdict(result), separators=(",", ":")))
    return result
