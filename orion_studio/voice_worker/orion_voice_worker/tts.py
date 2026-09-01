from __future__ import annotations

from dataclasses import dataclass
from typing import Callable, Protocol

import numpy as np


@dataclass(frozen=True)
class SpeechAudio:
    pcm: bytes
    sample_rate: int

    @property
    def samples(self) -> int:
        return len(self.pcm) // 2

    @property
    def duration_ms(self) -> int:
        return round(self.samples * 1_000 / self.sample_rate)


class TtsModel(Protocol):
    def generate(self, text: str, **options: object): ...


class ChatterboxSynthesizer:
    provider = "chatterbox-turbo"

    def __init__(
        self,
        model: str,
        model_loader: Callable[[str], TtsModel] | None = None,
    ) -> None:
        if model_loader is None:
            from mlx_audio.tts.utils import load

            model_loader = load
        self.model_name = model
        self._model = model_loader(model)

    def synthesize(self, text: str) -> SpeechAudio:
        chunks: list[np.ndarray] = []
        sample_rate: int | None = None
        for result in self._model.generate(text=text, stream=False):
            result_rate = int(result.sample_rate)
            if result_rate <= 0:
                raise RuntimeError("Chatterbox returned an invalid sample rate.")
            if sample_rate is None:
                sample_rate = result_rate
            elif result_rate != sample_rate:
                raise RuntimeError("Chatterbox changed sample rate during synthesis.")
            chunks.append(np.asarray(result.audio, dtype=np.float32).reshape(-1))

        if sample_rate is None or not chunks:
            raise RuntimeError("Chatterbox generated no audio.")
        waveform = np.concatenate(chunks)
        if waveform.size == 0 or not np.isfinite(waveform).all():
            raise RuntimeError("Chatterbox generated invalid audio.")
        pcm = (np.clip(waveform, -1.0, 1.0) * 32_767).astype("<i2").tobytes()
        return SpeechAudio(pcm=pcm, sample_rate=sample_rate)
