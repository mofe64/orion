from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class Transcript:
    text: str
    language: str | None
