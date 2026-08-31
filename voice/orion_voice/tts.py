"""Piper adapter for Orion's selected production voice."""

from __future__ import annotations

import audioop
from pathlib import Path
import wave


ORION_PLAYBACK_SAMPLE_RATE = 48_000
DEFAULT_PIPER_VOICE_NAME = "en_US-ryan-medium"
DEFAULT_PIPER_MODEL_DIRECTORY = Path(__file__).resolve().parents[1] / "models"
DEFAULT_PIPER_MODEL_PATH = (
    DEFAULT_PIPER_MODEL_DIRECTORY / f"{DEFAULT_PIPER_VOICE_NAME}.onnx"
)


class PiperSynthesizer:
    """Load one Piper voice and produce Orion's physical playback format."""

    def __init__(
        self,
        model_path: Path = DEFAULT_PIPER_MODEL_PATH,
        voice_loader=None,
    ) -> None:
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
