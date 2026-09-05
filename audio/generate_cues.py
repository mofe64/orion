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
PEAK = 0.32
VOICE_CUE_GAIN = {"voice_wake": 0.12, "voice_processing": 0.08}


def envelope(position: float, duration: float) -> float:
    attack = min(0.035, duration / 4.0)
    if position < attack:
        return math.sin((position / attack) * math.pi / 2.0) ** 2
    decay_position = (position - attack) / max(duration - attack, 1e-9)
    return max(0.0, (1.0 - decay_position) ** 1.4)


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


def phrase(notes: list[tuple[float, float]], gap: float = 0.018) -> list[int]:
    samples: list[int] = []
    for index, (frequency, duration) in enumerate(notes):
        samples.extend(tone(frequency, duration))
        if index + 1 < len(notes):
            samples.extend(silence(gap))
    return samples


def main() -> None:
    cues = {
        "notice_warm": [(523.251, 0.16), (659.255, 0.21)],
        "acknowledge_warm": [(659.255, 0.16), (783.991, 0.22)],
        "curious_rise": [(440.000, 0.14), (554.365, 0.15), (659.255, 0.22)],
        "agree_soft": [(523.251, 0.17), (659.255, 0.20)],
        "delight_warm": [(523.251, 0.12), (659.255, 0.13), (783.991, 0.24)],
        "settle_soft": [(659.255, 0.15), (523.251, 0.27)],
        "error_muted": [(392.000, 0.17), (349.228, 0.25)],
    }
    cues.update({"voice_wake": [(220.0, 0.20)], "voice_processing": [(180.0, 0.18)]})
    directory = Path(__file__).resolve().parent / "cues"
    for name, notes in cues.items():
        output = directory / f"{name}.wav"
        write_stereo_wav(output, [round(sample * VOICE_CUE_GAIN.get(name, 1.0)) for sample in phrase(notes)])
        print(f"Generated {output}")


if __name__ == "__main__":
    main()
