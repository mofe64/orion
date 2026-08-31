"""Persistent Unix-socket worker for local speech synthesis."""

from __future__ import annotations

import os
from pathlib import Path
import socket
import stat
import threading
from typing import Protocol

from .protocol import MAX_REQUEST_BYTES, SynthesisRequest, SynthesisResult


DEFAULT_TTS_SOCKET_PATH = Path("/tmp/orion-tts.sock")
DEFAULT_TTS_OUTPUT_DIRECTORY = Path("/tmp/orion-tts")


class Synthesizer(Protocol):
    def synthesize(self, text: str, output_path: Path) -> float: ...


class TtsWorker:
    def __init__(
        self,
        synthesizer: Synthesizer,
        socket_path: Path = DEFAULT_TTS_SOCKET_PATH,
        output_directory: Path = DEFAULT_TTS_OUTPUT_DIRECTORY,
    ) -> None:
        self._synthesizer = synthesizer
        self._socket_path = socket_path
        self._output_directory = output_directory
        self._server: socket.socket | None = None
        self._stopping = threading.Event()

    def serve_forever(self) -> None:
        self._bind()
        assert self._server is not None
        print(f"orion-tts: ready on {self._socket_path}", flush=True)
        try:
            while not self._stopping.is_set():
                try:
                    connection, _ = self._server.accept()
                except TimeoutError:
                    continue
                with connection:
                    self._serve_connection(connection)
        finally:
            self.close()

    def stop(self) -> None:
        self._stopping.set()

    def close(self) -> None:
        if self._server is not None:
            self._server.close()
            self._server = None
        try:
            if self._socket_path.is_socket():
                self._socket_path.unlink()
        except FileNotFoundError:
            pass

    def _bind(self) -> None:
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

        self._output_directory.mkdir(parents=True, exist_ok=True)
        self._server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._server.bind(str(self._socket_path))
        os.chmod(self._socket_path, 0o660)
        self._server.listen(4)
        self._server.settimeout(0.1)

    def _serve_connection(self, connection: socket.socket) -> None:
        request_id = 0
        output_path: Path | None = None
        try:
            line = self._read_request_line(connection)
            request = SynthesisRequest.from_json_line(line)
            request_id = request.request_id
            output_path = self._output_directory / f"speech-{request.request_id}.wav"
            self._synthesizer.synthesize(request.text, output_path)
            result = SynthesisResult.ready(request.request_id, output_path)
        except Exception as error:
            if output_path is not None:
                output_path.unlink(missing_ok=True)
            result = SynthesisResult.failed(request_id, str(error))

        try:
            connection.sendall(result.to_json_line())
        except (BrokenPipeError, ConnectionResetError):
            if output_path is not None:
                output_path.unlink(missing_ok=True)

    @staticmethod
    def _read_request_line(connection: socket.socket) -> bytes:
        received = bytearray()
        while len(received) <= MAX_REQUEST_BYTES:
            chunk = connection.recv(min(1024, MAX_REQUEST_BYTES + 1 - len(received)))
            if not chunk:
                break
            received.extend(chunk)
            if b"\n" in chunk:
                break
        if len(received) > MAX_REQUEST_BYTES:
            raise ValueError("TTS request is too large")
        line, separator, trailing = bytes(received).partition(b"\n")
        if not separator:
            raise ValueError("TTS request must end with a newline")
        if trailing:
            raise ValueError("TTS connection accepts one request")
        return line
