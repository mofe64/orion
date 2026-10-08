"""Opt-in, bounded wake diagnostics; only the writer thread touches disk."""
from collections import deque
from datetime import datetime, timezone
import json
import logging
import os
from pathlib import Path
import queue
import re
import shutil
import threading
import time
import wave

RATE = 16_000
WINDOW_SAMPLES = 4 * RATE
MAX_RECORDINGS = 50
_FOLDER_NAME = re.compile(r"\d{8}T\d{12}Z-(?:accepted|rejected)")


class WakeDebugRecorder:
    def __init__(self, directory, *, clock=time.monotonic):
        self.directory = Path(directory).expanduser()
        self.clock = clock
        self.audio = bytearray()
        self.chunks = deque(maxlen=50)  # four seconds of 80 ms chunks
        self.candidate = None
        self.opened_at = None
        self._queue = queue.Queue(maxsize=8)
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, name="wake-debug-writer", daemon=True)
        self._thread.start()

    @classmethod
    def from_environment(cls):
        directory = os.environ.get("ORION_WAKE_DEBUG_DIR")
        return cls(directory) if directory else None

    def capture_opened(self):
        self.audio.clear()
        self.chunks.clear()
        self.candidate = None
        self.opened_at = self.clock()

    def accept(self, pcm, chunks, position):
        self.audio.extend(pcm)
        del self.audio[:-WINDOW_SAMPLES * 2]
        self.chunks.extend(chunks)
        while self.chunks and self.chunks[0][0] <= position - WINDOW_SAMPLES:
            self.chunks.popleft()

    def begin(self, session_id, phase, score, position):
        self.candidate = {"session_id": session_id, "phase_at_candidate": phase,
                          "rustpotter_score": score, "candidate_position": position}

    def cancel(self):
        self.candidate = None

    def verdict(self, verdict, verifier):
        candidate, self.candidate = self.candidate, None
        if candidate is None or self._stop.is_set():
            return
        now = datetime.now(timezone.utc)
        pcm = bytes(self.audio)
        meta = {**candidate, "captured_at_utc": now.isoformat(),
                "sample_rate": RATE, "audio_start_sample": verifier.position - len(pcm) // 2,
                "audio_end_sample": verifier.position,
                "verdict": {"accepted": verdict.accepted, "score": verdict.score,
                            "decided_sample": verdict.decided_sample},
                "verifier_chunks": [{"end_sample": end, "score": score, "eligible": eligible}
                                    for end, score, eligible in self.chunks],
                "verifier_healthy": verifier.healthy,
                "verifier_chunk_seconds": list(verifier.chunk_seconds),
                "capture_open_seconds": None if self.opened_at is None else self.clock() - self.opened_at,
                "verifier_threshold": verifier.threshold,
                "verifier_lookback_samples": verifier.lookback,
                "verifier_deadline_samples": verifier.deadline}
        outcome = "accepted" if verdict.accepted else "rejected"
        folder = now.strftime("%Y%m%dT%H%M%S%fZ") + "-" + outcome
        try:
            self._queue.put_nowait((folder, pcm, meta))
        except queue.Full:
            logging.warning("Wake debug queue full; diagnostic recording dropped")

    def _write(self, folder, pcm, meta):
        self.directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        recordings = sorted(path for path in self.directory.iterdir()
                            if _FOLDER_NAME.fullmatch(path.name) and path.is_dir() and not path.is_symlink())
        for oldest in recordings[:max(0, len(recordings) - MAX_RECORDINGS + 1)]:
            shutil.rmtree(oldest)
        destination = self.directory / folder
        destination.mkdir(mode=0o700)
        with wave.open(str(destination / "audio.wav"), "wb") as output:
            output.setnchannels(1)
            output.setsampwidth(2)
            output.setframerate(RATE)
            output.writeframes(pcm)
        (destination / "meta.json").write_text(json.dumps(meta, indent=2) + "\n")

    def _run(self):
        while not self._stop.is_set() or not self._queue.empty():
            try:
                job = self._queue.get(timeout=0.1)
            except queue.Empty:
                continue
            try:
                self._write(*job)
            except Exception as error:
                logging.warning("Wake debug recording failed: %s", error)
            finally:
                self._queue.task_done()

    def close(self):
        self._stop.set()
        # Capture has already stopped. A stuck disk must not hold shutdown open.
        self._thread.join(timeout=2)


class CaptureReadStats:
    """Count read latency including dispatch to/from the capture worker thread."""
    def __init__(self, *, clock=time.monotonic):
        self.clock = clock
        self.started = clock()
        self.reads = self.slow = 0

    def accept(self, seconds):
        self.reads += 1
        self.slow += seconds > 0.040
        now = self.clock()
        if now - self.started >= 60:
            print(json.dumps({"event": "voice.capture_read_timing", "reads": self.reads,
                              "reads_over_40_ms": self.slow,
                              "window_seconds": round(now - self.started, 3)}), flush=True)
            self.started, self.reads, self.slow = now, 0, 0
