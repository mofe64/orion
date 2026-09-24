"""Pinned Piper Alba adapter for the Pi's 24 kHz speech protocol."""
from __future__ import annotations

import os
from pathlib import Path

import numpy as np

from .tts import SpeechAudio


MODEL_ID = "piper-alba-medium"
MODEL_FILE = "en_GB-alba-medium.onnx"
SOURCE_RATE = 22_050
OUTPUT_RATE = 24_000
MAX_AUDIO_SECONDS = 120


def is_piper_model(model: str) -> bool:
    """Recognize the pinned ID or an explicit folder containing its weights."""
    return model == MODEL_ID or (Path(model).is_dir() and (Path(model) / MODEL_FILE).is_file())


class PiperAlbaSynthesizer:
    provider = "piper-tts"

    def __init__(self, model: str):
        import sherpa_onnx
        from scipy.signal import resample_poly

        if not is_piper_model(model):
            raise ValueError("Piper model must be piper-alba-medium or its model folder")
        location = os.environ.get("ORION_PIPER_MODEL_DIR") if model == MODEL_ID else model
        if not location:
            raise ValueError("ORION_PIPER_MODEL_DIR is required for Piper Alba")
        root = Path(location)
        weights = root / MODEL_FILE
        tokens = root / "tokens.txt"
        data = root / "espeak-ng-data"
        if not weights.is_file() or not tokens.is_file() or not data.is_dir():
            raise ValueError(f"Piper Alba model is incomplete: {root}")
        threads = int(os.environ.get("ORION_TTS_THREADS", "3"))
        if not 1 <= threads <= 4:
            raise ValueError("TTS threads must be between one and four")
        config = sherpa_onnx.OfflineTtsConfig(
            model=sherpa_onnx.OfflineTtsModelConfig(
                vits=sherpa_onnx.OfflineTtsVitsModelConfig(
                    model=str(weights), tokens=str(tokens), data_dir=str(data),
                ),
                num_threads=threads,
            ),
            max_num_sentences=1,
        )
        if not config.validate():
            raise ValueError(f"Piper Alba configuration is invalid: {root}")
        self.model = sherpa_onnx.OfflineTts(config)
        if self.model.sample_rate != SOURCE_RATE:
            raise ValueError("Piper Alba must produce 22,050 Hz audio")
        self.model_name = model
        self.resample = resample_poly

    def stream(self, text: str):
        # The coordinator holds at least six seconds of audio before playback.
        # Completing this fast model's sentence first avoids a producer thread
        # while still supplying bounded chunks to the existing wire protocol.
        generated = self.model.generate(text, sid=0, speed=1.0)
        if generated.sample_rate != SOURCE_RATE:
            raise ValueError("Piper Alba changed its sample rate")
        audio = np.asarray(generated.samples, dtype=np.float32).reshape(-1)
        if not audio.size or audio.size > SOURCE_RATE * MAX_AUDIO_SECONDS or not np.isfinite(audio).all():
            raise ValueError("Piper Alba produced invalid audio")
        # 22,050 * 160 / 147 = 24,000 exactly. Resample the whole sentence so
        # filter boundaries do not introduce clicks between transport chunks.
        output = self.resample(audio, 160, 147)
        if not np.isfinite(output).all():
            raise ValueError("Piper Alba resampling produced invalid audio")
        pcm = (np.clip(output, -1, 1) * 32_767).astype("<i2").tobytes()
        for offset in range(0, len(pcm), OUTPUT_RATE * 4):
            yield SpeechAudio(pcm[offset:offset + OUTPUT_RATE * 4], OUTPUT_RATE)
