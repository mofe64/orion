"""Streaming openWakeWord phrase verifier for Rustpotter wake candidates.

The feature path is adapted from dscripka/openWakeWord v0.6.0 ``utils.py``
(Apache-2.0, Copyright 2023 David Scripka):
https://github.com/dscripka/openWakeWord/blob/v0.6.0/openwakeword/utils.py
It streams 80 ms chunks exactly as the verifier was trained and evaluated; it
never computes whole-clip mel spectrograms.
"""
from __future__ import annotations

from collections import deque
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import time

import numpy as np

RATE = 16_000
CHUNK = 1_280  # 80 ms, one embedding frame
MEL_CONTEXT = 480  # samples of overlap before each chunk
EMBEDDING_WINDOW = 76  # mel frames per embedding
FEATURE_WINDOW = 16  # embeddings per classifier input
FILES = ("melspectrogram.onnx", "embedding_model.onnx", "hey_orion_verifier.onnx")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def onnx_session(path: Path):
    import onnxruntime as ort
    options = ort.SessionOptions()
    options.intra_op_num_threads = options.inter_op_num_threads = 1
    return ort.InferenceSession(str(path), sess_options=options, providers=["CPUExecutionProvider"])


@dataclass(frozen=True)
class Verdict:
    accepted: bool
    score: float | None
    decided_sample: int


class StreamingScorer:
    """Score every 80 ms of continuous PCM16; the first frames after reset are ineligible."""

    def __init__(self, model_dir: Path, warmup_frames: int, verify_hashes: dict[str, str] | None = None) -> None:
        for name in FILES:
            path = model_dir / name
            if not path.is_file():
                raise RuntimeError(f"Wake verifier file is missing: {path}")
            if verify_hashes is not None and sha256(path) != verify_hashes.get(name):
                raise RuntimeError(f"Wake verifier file does not match its recorded hash: {path}")
        self._mel = onnx_session(model_dir / "melspectrogram.onnx")
        self._embedding = onnx_session(model_dir / "embedding_model.onnx")
        self._classifier = onnx_session(model_dir / "hey_orion_verifier.onnx")
        self._classifier_input = self._classifier.get_inputs()[0].name
        self.warmup_frames = warmup_frames
        self.reset()

    def reset(self) -> None:
        self._raw = np.empty(0, dtype=np.int16)
        self._pending = np.empty(0, dtype=np.int16)
        self._mels = np.ones((EMBEDDING_WINDOW, 32), dtype=np.float32)
        self._features = np.zeros((FEATURE_WINDOW, 96), dtype=np.float32)
        self.frames = 0
        self.samples = 0  # stream position after the last complete chunk

    def feed(self, pcm: np.ndarray) -> list[tuple[int, float, bool]]:
        """Return (chunk end sample, score, eligible) for each completed 80 ms chunk."""
        self._pending = np.concatenate((self._pending, pcm.astype(np.int16, copy=False)))
        scores = []
        while len(self._pending) >= CHUNK:
            chunk, self._pending = self._pending[:CHUNK], self._pending[CHUNK:]
            self._raw = np.concatenate((self._raw, chunk))[-(CHUNK + MEL_CONTEXT):]
            mel = self._mel.run(None, {"input": self._raw.astype(np.float32)[None]})[0].squeeze() / 10 + 2
            self._mels = np.vstack((self._mels, mel))[-970:]
            window = self._mels[-EMBEDDING_WINDOW:].astype(np.float32)[None, :, :, None]
            embedding = self._embedding.run(None, {"input_1": window})[0].squeeze()
            self._features = np.vstack((self._features, embedding))[-120:]
            self.frames += 1
            self.samples += CHUNK
            features = self._features[-FEATURE_WINDOW:].astype(np.float32)[None]
            score = float(self._classifier.run(None, {self._classifier_input: features})[0].reshape(-1)[0])
            scores.append((self.samples, score, self.frames > self.warmup_frames))
        return scores


class AcousticVerifier:
    """Confirm a Rustpotter candidate when the phrase score crosses a threshold near it.

    Rule, identical to the Gate 2 session simulator: accept a candidate at
    sample ``c`` when an eligible score >= ``threshold`` occurs between
    ``c - lookback`` and ``c + deadline``; otherwise reject at ``c + deadline``.
    Scores from before ``c`` accept immediately at ``c``.
    """

    provider = "openwakeword"

    def __init__(self, model_dir: Path, *, scorer: StreamingScorer | None = None, clock=time.perf_counter) -> None:
        config = json.loads((model_dir / "config.json").read_text())
        self.model_dir = model_dir
        self.model_name = "hey_orion_verifier.onnx"
        self.threshold = float(config["threshold"])
        self.lookback = round(float(config["lookback_seconds"]) * RATE)
        self.deadline = round(float(config["deadline_seconds"]) * RATE)
        if not 0 < self.threshold < 1 or self.lookback < 0 or self.deadline < 0:
            raise RuntimeError("Invalid wake verifier configuration")
        self.scorer = scorer or StreamingScorer(model_dir, int(config["warmup_frames"]), config["files"])
        self.clock = clock
        self.history: deque[tuple[int, float]] = deque()
        self.chunk_seconds: deque[float] = deque(maxlen=250)  # about 20 s of chunks
        self.healthy = True
        self.candidate: int | None = None

    def describe(self) -> dict:
        return {"provider": self.provider, "model": self.model_name, "threshold": self.threshold,
                "lookbackSeconds": self.lookback / RATE, "deadlineSeconds": self.deadline / RATE,
                "active": self.healthy}

    @property
    def position(self) -> int:
        return self.scorer.samples + len(self.scorer._pending)

    def reset(self) -> None:
        """Restart after a capture discontinuity; warmup applies again."""
        self.scorer.reset()
        self.history.clear()
        self.candidate = None

    def feed(self, pcm: np.ndarray) -> list[tuple[int, float, bool]]:
        started = self.clock()
        chunks = self.scorer.feed(pcm)
        for end, score, eligible in chunks:
            if eligible:
                self.history.append((end, score))
            self.chunk_seconds.append(self.clock() - started)
            started = self.clock()
        keep_from = self.position - self.lookback - 2 * RATE
        while self.history and self.history[0][0] < keep_from:
            self.history.popleft()
        self._check_health()
        # Diagnostics observe every score, including warmup; decision history
        # above keeps the same eligibility rule.
        return chunks

    def _check_health(self) -> None:
        # Real time allows 80 ms per chunk. Sustained use of more than half of it
        # would starve capture, so fall back to the unverified path for this run.
        if self.healthy and len(self.chunk_seconds) == self.chunk_seconds.maxlen:
            slow = sum(1 for seconds in self.chunk_seconds if seconds > 0.040)
            if slow > len(self.chunk_seconds) // 10:
                self.healthy = False

    def begin(self) -> None:
        """Start verifying a candidate emitted at the current stream position."""
        self.candidate = self.position

    def verdict(self) -> Verdict | None:
        """Return a decision once available; None while the candidate is still pending."""
        if self.candidate is None:
            return None
        start, end = self.candidate - self.lookback, self.candidate + self.deadline
        best = None
        for sample, score in self.history:
            if start <= sample <= min(end, self.position):
                best = score if best is None else max(best, score)
                if score >= self.threshold:
                    decided = max(sample, self.candidate)
                    self.candidate = None
                    return Verdict(True, score, decided)
        if self.position >= end:
            self.candidate = None
            return Verdict(False, best, end)
        return None

    def cancel(self) -> None:
        self.candidate = None
