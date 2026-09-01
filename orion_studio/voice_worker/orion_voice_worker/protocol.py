from __future__ import annotations

from dataclasses import dataclass
import json
from typing import Any

from . import FRAME_SAMPLES, PROTOCOL_VERSION, SAMPLE_RATE


class ProtocolError(ValueError):
    """A client message violated the versioned worker protocol."""


@dataclass(frozen=True)
class ClientHello:
    token: str


def parse_json_message(raw: str) -> dict[str, Any]:
    try:
        message = json.loads(raw)
    except json.JSONDecodeError as error:
        raise ProtocolError("Control messages must be valid JSON.") from error
    if not isinstance(message, dict) or not isinstance(message.get("type"), str):
        raise ProtocolError("Control messages require a string type.")
    return message


def parse_hello(raw: str) -> ClientHello:
    message = parse_json_message(raw)
    expected = {
        "type": "hello",
        "protocol": PROTOCOL_VERSION,
        "sampleRate": SAMPLE_RATE,
        "channels": 1,
        "encoding": "pcm_s16le",
        "frameSamples": FRAME_SAMPLES,
    }
    for key, value in expected.items():
        if message.get(key) != value:
            raise ProtocolError(f"Handshake field {key} must be {value!r}.")
    token = message.get("token")
    if not isinstance(token, str) or not token:
        raise ProtocolError("Handshake requires a worker token.")
    return ClientHello(token=token)


def event(message_type: str, **fields: object) -> str:
    return json.dumps({"type": message_type, **fields}, separators=(",", ":"))
