from __future__ import annotations

from dataclasses import dataclass
import re


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


def speech_segments(text: str, limit: int = 160):
    """Bound decoder context, preferring sentence ends and then word boundaries."""
    for sentence in re.split(r'(?<=[.!?])\s+|\n+', text.strip()):
        while sentence:
            if len(sentence) <= limit:
                yield sentence
                break
            cut = sentence.rfind(" ", 0, limit + 1)
            if cut <= 0:
                cut = limit
            yield sentence[:cut]
            sentence = sentence[cut:].lstrip()
