from __future__ import annotations

from dataclasses import dataclass

from . import SAMPLE_RATE


@dataclass(frozen=True)
class Transcript:
    text: str
    language: str | None


class Qwen3AsrTranscriber:
    provider = "qwen3-asr"

    def __init__(self, model: str) -> None:
        try:
            import numpy as np
            from mlx_qwen3_asr import Session
        except ImportError as error:
            raise RuntimeError(
                "mlx-qwen3-asr is not installed. The current Orion adapter requires an Apple Silicon Mac."
            ) from error
        self._np = np
        self.model_name = model
        self._session = Session(model=model)

    def transcribe(self, pcm: bytes) -> Transcript:
        audio = self._np.frombuffer(pcm, dtype="<i2").astype(self._np.float32) / 32_768.0
        result = self._session.transcribe((audio, SAMPLE_RATE))
        return Transcript(
            text=str(result.text).strip(),
            language=str(result.language) if result.language else None,
        )
