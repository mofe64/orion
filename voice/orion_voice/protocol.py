"""JSON-line protocol shared by Orion's local TTS worker and its clients."""

from __future__ import annotations

from dataclasses import asdict, dataclass
import json
from pathlib import Path
from typing import Any


MAX_SPEECH_TEXT_BYTES = 2_000
MAX_REQUEST_BYTES = 4_096


@dataclass(frozen=True)
class SynthesisRequest:
    request_id: int
    text: str

    @classmethod
    def from_json_line(cls, line: bytes) -> "SynthesisRequest":
        if not line or len(line) > MAX_REQUEST_BYTES:
            raise ValueError("TTS request is empty or too large")
        try:
            value: Any = json.loads(line)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise ValueError(f"TTS request is not valid JSON: {error}") from error
        if not isinstance(value, dict):
            raise ValueError("TTS request must be a JSON object")

        request_id = value.get("request_id")
        text = value.get("text")
        if not isinstance(request_id, int) or isinstance(request_id, bool) or request_id <= 0:
            raise ValueError("TTS request_id must be a positive integer")
        if not isinstance(text, str):
            raise ValueError("TTS text must be a string")
        text = text.strip()
        if not text:
            raise ValueError("TTS text cannot be empty")
        if "\n" in text or "\r" in text:
            raise ValueError("TTS text cannot contain line breaks")
        if len(text.encode("utf-8")) > MAX_SPEECH_TEXT_BYTES:
            raise ValueError(
                f"TTS text cannot exceed {MAX_SPEECH_TEXT_BYTES} UTF-8 bytes"
            )
        return cls(request_id=request_id, text=text)

    def to_json_line(self) -> bytes:
        return (json.dumps(asdict(self), separators=(",", ":")) + "\n").encode()


@dataclass(frozen=True)
class SynthesisResult:
    request_id: int
    state: str
    wav_path: str | None = None
    error: str | None = None

    @classmethod
    def ready(cls, request_id: int, wav_path: Path) -> "SynthesisResult":
        return cls(request_id=request_id, state="ready", wav_path=str(wav_path.resolve()))

    @classmethod
    def failed(cls, request_id: int, error: str) -> "SynthesisResult":
        return cls(request_id=request_id, state="failed", error=error)

    def to_json_line(self) -> bytes:
        return (json.dumps(asdict(self), separators=(",", ":")) + "\n").encode()
