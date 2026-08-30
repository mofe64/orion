#!/usr/bin/env python3

"""Generate Orion's original, redistributable local WAV cues."""

from __future__ import annotations

import math
import struct
import wave
from pathlib import Path


SAMPLE_RATE = 48_000
CHANNELS = 2
SAMPLE_WIDTH_BYTES = 2
PEAK = 0.50


def envelope(position: float, duration: float) -> float:
    fade = min(0.025, duration / 3.0)
    if position < fade:
        return math.sin((position / fade) * math.pi / 2.0) ** 2
    if position > duration - fade:
        remaining = max(0.0, duration - position)
        return math.sin((remaining / fade) * math.pi / 2.0) ** 2
    return 1.0


def tone(frequency_hz: float, duration_seconds: float) -> list[int]:
    frames = round(duration_seconds * SAMPLE_RATE)
    samples: list[int] = []
    for frame in range(frames):
        time_seconds = frame / SAMPLE_RATE
        fundamental = math.sin(2.0 * math.pi * frequency_hz * time_seconds)
        harmonic = 0.12 * math.sin(4.0 * math.pi * frequency_hz * time_seconds)
        value = PEAK * envelope(time_seconds, duration_seconds) * (fundamental + harmonic)
        samples.append(round(max(-1.0, min(1.0, value)) * 32_767))
    return samples


def silence(duration_seconds: float) -> list[int]:
    return [0] * round(duration_seconds * SAMPLE_RATE)


def write_stereo_wav(path: Path, mono_samples: list[int]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(path), "wb") as output:
        output.setnchannels(CHANNELS)
        output.setsampwidth(SAMPLE_WIDTH_BYTES)
        output.setframerate(SAMPLE_RATE)
        output.writeframes(
            b"".join(struct.pack("<hh", sample, sample) for sample in mono_samples)
        )


def main() -> None:
    output = Path(__file__).resolve().parent / "cues" / "acknowledge.wav"
    samples = (
        tone(659.255, 0.175)
        + silence(0.025)
        + tone(783.991, 0.220)
    )
    write_stereo_wav(output, samples)
    print(f"Generated {output}")


if __name__ == "__main__":
    main()
