from __future__ import annotations

from dataclasses import dataclass
from typing import Callable, Protocol

import numpy as np


TARGET_PEAK = 0.85
MAXIMUM_GAIN = 4.0
MINIMUM_AUDIBLE_PEAK = 1e-4


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

    def stream(self, text: str):
        sample_rate = None
        gain = None
        audible = False
        for result in self._model.generate(text=text, stream=True, streaming_interval=0.8):
            result_rate = int(result.sample_rate)
            if result_rate <= 0:
                raise RuntimeError("Chatterbox returned an invalid sample rate.")
            if sample_rate is not None and result_rate != sample_rate:
                raise RuntimeError("Chatterbox changed sample rate during synthesis.")
            sample_rate = result_rate
            waveform = np.asarray(result.audio, dtype=np.float32).reshape(-1)
            if waveform.size == 0 or not np.isfinite(waveform).all():
                raise RuntimeError("Chatterbox generated invalid audio.")
            peak = float(np.max(np.abs(waveform)))
            if peak >= MINIMUM_AUDIBLE_PEAK:
                audible = True
                if gain is None:
                    gain = min(TARGET_PEAK / peak, MAXIMUM_GAIN) if peak < TARGET_PEAK else 1.0
            # Freeze gain for the utterance; independent chunk normalization pumps volume.
            pcm = (np.clip(waveform * (gain or 1.0), -1.0, 1.0) * 32_767).astype("<i2").tobytes()
            # Transport chunks have a fixed upper bound even if the model emits a large tail.
            for offset in range(0, len(pcm), sample_rate * 2):
                yield SpeechAudio(pcm=pcm[offset:offset + sample_rate * 2], sample_rate=sample_rate)
        if sample_rate is None:
            raise RuntimeError("Chatterbox generated no audio.")
        if not audible:
            raise RuntimeError("Chatterbox generated inaudible audio.")

    def synthesize(self, text: str) -> SpeechAudio:
        chunks = list(self.stream(text))
        return SpeechAudio(b"".join(chunk.pcm for chunk in chunks), chunks[0].sample_rate)
