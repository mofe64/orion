"""Transient speech segmentation and local Moonshine transcription."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import time
from typing import Callable, Sequence

import numpy as np


SPEECH_SAMPLE_RATE = 16_000
DEFAULT_ASR_MODEL_NAME = "sherpa-onnx-moonshine-tiny-en-int8"
DEFAULT_ASR_MODEL_DIRECTORY = (
    Path(__file__).resolve().parents[1] / "models" / "asr" / DEFAULT_ASR_MODEL_NAME
)
DEFAULT_VAD_MODEL_PATH = (
    Path(__file__).resolve().parents[1] / "models" / "vad" / "silero_vad.onnx"
)


@dataclass(frozen=True)
class Transcription:
    text: str
    audio_seconds: float
    inference_seconds: float


class MoonshineTranscriber:
    """Load Moonshine Tiny once and transcribe complete in-memory utterances."""

    def __init__(
        self,
        model_directory: Path = DEFAULT_ASR_MODEL_DIRECTORY,
        *,
        num_threads: int = 2,
        recognizer_factory=None,
        monotonic: Callable[[], float] = time.monotonic,
    ) -> None:
        model_directory = Path(model_directory)
        paths = {
            "preprocessor": model_directory / "preprocess.onnx",
            "encoder": model_directory / "encode.int8.onnx",
            "uncached_decoder": model_directory / "uncached_decode.int8.onnx",
            "cached_decoder": model_directory / "cached_decode.int8.onnx",
            "tokens": model_directory / "tokens.txt",
        }
        missing = [str(path) for path in paths.values() if not path.is_file()]
        if missing:
            raise FileNotFoundError(
                "Moonshine ASR model is incomplete. Run voice/install-models.sh. "
                f"Missing: {', '.join(missing)}"
            )
        if num_threads <= 0:
            raise ValueError("ASR thread count must be positive")

        if recognizer_factory is None:
            import sherpa_onnx

            recognizer_factory = sherpa_onnx.OfflineRecognizer.from_moonshine
        self._recognizer = recognizer_factory(
            **{name: str(path) for name, path in paths.items()},
            num_threads=num_threads,
            decoding_method="greedy_search",
            provider="cpu",
        )
        self._monotonic = monotonic

    def transcribe(self, samples: Sequence[float]) -> Transcription:
        audio = np.asarray(samples, dtype=np.float32).reshape(-1)
        if audio.size == 0:
            raise ValueError("cannot transcribe an empty utterance")
        if not np.isfinite(audio).all():
            raise ValueError("utterance contains non-finite samples")

        stream = self._recognizer.create_stream()
        stream.accept_waveform(SPEECH_SAMPLE_RATE, audio)
        started = self._monotonic()
        self._recognizer.decode_stream(stream)
        inference_seconds = self._monotonic() - started
        text = stream.result.text.strip()
        return Transcription(
            text=text,
            audio_seconds=audio.size / SPEECH_SAMPLE_RATE,
            inference_seconds=inference_seconds,
        )


class SherpaSpeechSegmenter:
    """Use Silero VAD to emit one or more completed speech segments."""

    def __init__(
        self,
        model_path: Path = DEFAULT_VAD_MODEL_PATH,
        *,
        threshold: float = 0.5,
        min_silence_seconds: float = 0.8,
        min_speech_seconds: float = 0.25,
        max_speech_seconds: float = 10.0,
        num_threads: int = 1,
        vad_factory=None,
    ) -> None:
        model_path = Path(model_path)
        if not model_path.is_file():
            raise FileNotFoundError(
                "Silero VAD model is missing. Run voice/install-models.sh. "
                f"Missing: {model_path}"
            )
        if not 0 < threshold < 1:
            raise ValueError("VAD threshold must be between zero and one")
        if min_silence_seconds <= 0 or min_speech_seconds <= 0:
            raise ValueError("VAD speech and silence durations must be positive")
        if max_speech_seconds <= min_speech_seconds:
            raise ValueError("VAD maximum speech duration is too short")
        if num_threads <= 0:
            raise ValueError("VAD thread count must be positive")

        arguments = {
            "model": str(model_path),
            "threshold": threshold,
            "min_silence_duration": min_silence_seconds,
            "min_speech_duration": min_speech_seconds,
            "max_speech_duration": max_speech_seconds,
            "sample_rate": SPEECH_SAMPLE_RATE,
            "num_threads": num_threads,
        }
        if vad_factory is None:
            import sherpa_onnx

            config = sherpa_onnx.VadModelConfig()
            config.silero_vad.model = arguments["model"]
            config.silero_vad.threshold = threshold
            config.silero_vad.min_silence_duration = min_silence_seconds
            config.silero_vad.min_speech_duration = min_speech_seconds
            config.silero_vad.max_speech_duration = max_speech_seconds
            config.sample_rate = SPEECH_SAMPLE_RATE
            config.num_threads = num_threads
            self._window_size = config.silero_vad.window_size
            self._vad = sherpa_onnx.VoiceActivityDetector(
                config, buffer_size_in_seconds=max_speech_seconds + 2
            )
        else:
            self._vad, self._window_size = vad_factory(**arguments)

        if self._window_size <= 0:
            raise ValueError("VAD window size must be positive")
        self._pending = np.empty(0, dtype=np.float32)

    def accept_pcm16(self, pcm: bytes) -> list[np.ndarray]:
        if len(pcm) % 2:
            raise ValueError("microphone PCM chunk has an incomplete 16-bit sample")
        samples = np.frombuffer(pcm, dtype="<i2").astype(np.float32) / 32768.0
        self._pending = np.concatenate((self._pending, samples))
        while self._pending.size >= self._window_size:
            self._vad.accept_waveform(self._pending[: self._window_size])
            self._pending = self._pending[self._window_size :]

        segments: list[np.ndarray] = []
        while not self._vad.empty():
            segments.append(np.asarray(self._vad.front.samples, dtype=np.float32).copy())
            self._vad.pop()
        return segments

    def reset(self) -> None:
        self._vad.reset()
        self._pending = np.empty(0, dtype=np.float32)
