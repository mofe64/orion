"""Offline wake-word detection and event publication for Orion."""

from __future__ import annotations

from contextlib import suppress
from dataclasses import asdict, dataclass
import json
import os
from pathlib import Path
import socket
import stat
import subprocess
from typing import Sequence


WAKE_SAMPLE_RATE = 16_000
WAKE_CHUNK_MILLISECONDS = 100
DEFAULT_CAPTURE_DEVICE = "plughw:CARD=seeed2micvoicec,DEV=0"
DEFAULT_WAKE_SOCKET_PATH = Path("/tmp/orion-wake.sock")
DEFAULT_WAKE_MODEL_NAME = (
    "sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01"
)
DEFAULT_WAKE_MODEL_DIRECTORY = (
    Path(__file__).resolve().parents[1] / "models" / "wake" / DEFAULT_WAKE_MODEL_NAME
)


@dataclass(frozen=True)
class WakeEvent:
    event_id: int
    event: str
    phrase: str

    def to_json_line(self) -> bytes:
        return (json.dumps(asdict(self), separators=(",", ":")) + "\n").encode()


class SherpaWakeDetector:
    """Decode arbitrary configured keywords with Sherpa's streaming KWS model."""

    def __init__(
        self,
        model_directory: Path = DEFAULT_WAKE_MODEL_DIRECTORY,
        *,
        num_threads: int = 2,
        keywords_score: float = 1.5,
        keywords_threshold: float = 0.25,
        spotter_factory=None,
    ) -> None:
        model_directory = Path(model_directory)
        paths = {
            "tokens": model_directory / "tokens.txt",
            "encoder": model_directory
            / "encoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx",
            "decoder": model_directory
            / "decoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx",
            "joiner": model_directory
            / "joiner-epoch-12-avg-2-chunk-16-left-64.int8.onnx",
            "keywords_file": model_directory / "orion_keywords.txt",
        }
        missing = [str(path) for path in paths.values() if not path.is_file()]
        if missing:
            raise FileNotFoundError(
                "Wake-word model is incomplete. Run voice/install-models.sh. "
                f"Missing: {', '.join(missing)}"
            )
        if num_threads <= 0:
            raise ValueError("wake-word thread count must be positive")
        if keywords_score <= 0:
            raise ValueError("wake-word keyword score must be positive")
        if not 0 < keywords_threshold < 1:
            raise ValueError("wake-word threshold must be between zero and one")

        if spotter_factory is None:
            import sherpa_onnx

            spotter_factory = sherpa_onnx.KeywordSpotter
        self._spotter = spotter_factory(
            **{name: str(path) for name, path in paths.items()},
            num_threads=num_threads,
            max_active_paths=4,
            keywords_score=keywords_score,
            keywords_threshold=keywords_threshold,
            num_trailing_blanks=2,
            provider="cpu",
        )
        self._stream = self._spotter.create_stream()

    def accept_pcm16(self, pcm: bytes) -> list[str]:
        if len(pcm) % 2:
            raise ValueError("microphone PCM chunk has an incomplete 16-bit sample")
        import numpy as np

        samples = np.frombuffer(pcm, dtype="<i2").astype(np.float32) / 32768.0
        return self.accept_samples(samples)

    def accept_samples(self, samples: Sequence[float]) -> list[str]:
        self._stream.accept_waveform(WAKE_SAMPLE_RATE, samples)
        while self._spotter.is_ready(self._stream):
            self._spotter.decode_stream(self._stream)

        result = self._spotter.get_result(self._stream)
        if not result:
            return []
        phrase = result if isinstance(result, str) else result.keyword
        self._spotter.reset_stream(self._stream)
        return [phrase]


class AlsaPcmCapture:
    """Read transient mono PCM from Orion's stable ALSA capture device."""

    def __init__(self, device: str = DEFAULT_CAPTURE_DEVICE) -> None:
        if not device.strip():
            raise ValueError("ALSA capture device cannot be empty")
        self._device = device
        self._process: subprocess.Popen[bytes] | None = None
        self._chunk_bytes = WAKE_SAMPLE_RATE * 2 * WAKE_CHUNK_MILLISECONDS // 1_000

    def command(self) -> list[str]:
        return [
            "arecord",
            "-q",
            "-D",
            self._device,
            "-t",
            "raw",
            "-f",
            "S16_LE",
            "-r",
            str(WAKE_SAMPLE_RATE),
            "-c",
            "1",
        ]

    def open(self) -> None:
        if self._process is not None:
            raise RuntimeError("microphone capture is already open")
        self._process = subprocess.Popen(self.command(), stdout=subprocess.PIPE)

    def read(self) -> bytes:
        if self._process is None or self._process.stdout is None:
            raise RuntimeError("microphone capture is not open")
        chunk = self._process.stdout.read(self._chunk_bytes)
        if len(chunk) != self._chunk_bytes:
            raise RuntimeError("ALSA microphone capture ended unexpectedly")
        return chunk

    def close(self) -> None:
        if self._process is None:
            return
        process = self._process
        self._process = None
        if process.stdout is not None:
            process.stdout.close()
        with suppress(ProcessLookupError):
            process.terminate()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


class WakeEventPublisher:
    """Publish wake events to local subscribers over one Unix socket."""

    def __init__(self, socket_path: Path = DEFAULT_WAKE_SOCKET_PATH) -> None:
        self._socket_path = Path(socket_path)
        self._server: socket.socket | None = None
        self._subscribers: list[socket.socket] = []
        self._next_event_id = 1

    def open(self) -> None:
        if self._server is not None:
            raise RuntimeError("wake event publisher is already open")
        try:
            mode = self._socket_path.lstat().st_mode
        except FileNotFoundError:
            pass
        else:
            if not stat.S_ISSOCK(mode):
                raise RuntimeError(
                    f"refusing to replace non-socket path: {self._socket_path}"
                )
            self._socket_path.unlink()

        self._server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._server.bind(str(self._socket_path))
        os.chmod(self._socket_path, 0o660)
        self._server.listen(4)
        self._server.setblocking(False)

    def accept_pending(self) -> None:
        if self._server is None:
            raise RuntimeError("wake event publisher is not open")
        while True:
            try:
                subscriber, _ = self._server.accept()
            except BlockingIOError:
                return
            subscriber.settimeout(0.1)
            self._subscribers.append(subscriber)

    def publish(self, phrase: str) -> WakeEvent:
        self.accept_pending()
        event = WakeEvent(self._next_event_id, "wake_word", phrase)
        self._next_event_id += 1
        payload = event.to_json_line()
        connected: list[socket.socket] = []
        for subscriber in self._subscribers:
            try:
                subscriber.sendall(payload)
            except (BrokenPipeError, ConnectionResetError, TimeoutError, OSError):
                subscriber.close()
            else:
                connected.append(subscriber)
        self._subscribers = connected
        print(payload.decode().rstrip(), flush=True)
        return event

    def close(self) -> None:
        for subscriber in self._subscribers:
            subscriber.close()
        self._subscribers.clear()
        if self._server is not None:
            self._server.close()
            self._server = None
        try:
            if self._socket_path.is_socket():
                self._socket_path.unlink()
        except FileNotFoundError:
            pass


class WakeWorker:
    def __init__(
        self,
        detector: SherpaWakeDetector,
        capture: AlsaPcmCapture,
        publisher: WakeEventPublisher,
    ) -> None:
        self._detector = detector
        self._capture = capture
        self._publisher = publisher

    def serve_forever(self) -> None:
        self._publisher.open()
        try:
            self._capture.open()
            print("orion-wake: listening for HEY ORION", flush=True)
            while True:
                self._publisher.accept_pending()
                for phrase in self._detector.accept_pcm16(self._capture.read()):
                    self._publisher.publish(phrase)
        finally:
            self.close()

    def close(self) -> None:
        self._capture.close()
        self._publisher.close()


def wait_for_wake(socket_path: Path = DEFAULT_WAKE_SOCKET_PATH) -> WakeEvent:
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as subscriber:
        subscriber.connect(str(socket_path))
        line = subscriber.makefile("rb").readline(4_096)
    if not line.endswith(b"\n"):
        raise RuntimeError("wake worker closed without publishing an event")
    value = json.loads(line)
    event = WakeEvent(**value)
    if event.event != "wake_word" or event.event_id <= 0 or not event.phrase:
        raise RuntimeError("wake worker published an invalid event")
    return event
