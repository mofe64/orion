#!/usr/bin/env python3

"""Build Orion's WAV cues from the Reachy Mini Emotions Library.

Each cue is one short gesture cut from a library recording and lowered in pitch.
"""

from __future__ import annotations

import argparse
from pathlib import Path

import librosa
import numpy as np
import soundfile


SAMPLE_RATE = 48_000
PITCH_STEPS = -3
FADE_IN_SECONDS = 0.02
FADE_OUT_SECONDS = 0.10
# Cue name: (library recording, start seconds, end seconds, peak dBFS).
CUES = {
    "voice_wake": ("inquiring1", 1.15, 1.43, -12),
    "voice_processing": ("understanding2", 1.06, 1.45, -18),
    "listen_hm": ("attentive1", 0.16, 0.68, -18),
    "wake_chirp": ("wake-mini-up", 3.57, 6.13, -20),
    "sleep_sigh": ("sleep1", 3.65, 4.55, -20),
    "yawn_hum": ("tired1", 1.10, 3.69, -20),
    "think_hum": ("thoughtful1", 1.97, 3.38, -20),
    "notice_warm": ("welcoming2", 1.13, 2.47, -15),
    "acknowledge_warm": ("helpful1", 0.91, 2.30, -15),
    "curious_rise": ("inquiring2", 1.24, 2.04, -15),
    "agree_soft": ("understanding1", 1.27, 2.23, -15),
    "disagree_soft": ("no1", 0.63, 1.17, -15),
    "delight_warm": ("cheerful1", 0.31, 1.24, -15),
    "success_bright": ("success2", 0.71, 1.37, -15),
    "laugh_light": ("laughing2", 0.79, 1.48, -15),
    "surprise_pop": ("surprised1", 0.74, 1.00, -15),
    "settle_soft": ("grateful1", 0.37, 1.01, -15),
    "didnt_catch": ("incomprehensible2", 1.16, 1.61, -15),
    "error_muted": ("confused1", 0.62, 1.64, -15),
    "sorry_low": ("downcast1", 0.84, 3.31, -15),
}
# The wake cue stays short: the person continues to speak after it.
PADDING = {"voice_wake": (0.02, 0.06)}


def build(source: Path, name: str) -> np.ndarray:
    recording, start, end, peak = CUES[name]
    samples, _ = librosa.load(source / f"{recording}.ogg", sr=SAMPLE_RATE, mono=True)
    before, after = PADDING.get(name, (0.06, 0.12))
    first = round(max(0.0, start - before) * SAMPLE_RATE)
    last = round(min(len(samples) / SAMPLE_RATE, end + after) * SAMPLE_RATE)
    cue = librosa.effects.pitch_shift(
        samples[first:last], sr=SAMPLE_RATE, n_steps=PITCH_STEPS, res_type="soxr_hq"
    )
    fade_in = round(FADE_IN_SECONDS * SAMPLE_RATE)
    fade_out = round(FADE_OUT_SECONDS * SAMPLE_RATE)
    cue[:fade_in] *= np.linspace(0.0, 1.0, fade_in) ** 2
    cue[-fade_out:] *= np.linspace(1.0, 0.0, fade_out) ** 2
    return cue * (10 ** (peak / 20) / np.abs(cue).max())


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, help="Directory that contains the library .ogg files")
    source = parser.parse_args().source
    directory = Path(__file__).resolve().parent / "cues"
    directory.mkdir(parents=True, exist_ok=True)
    for name in CUES:
        cue = build(source, name)
        output = directory / f"{name}.wav"
        soundfile.write(output, np.column_stack([cue, cue]), SAMPLE_RATE, subtype="PCM_16")
        print(f"Built {output}")


if __name__ == "__main__":
    main()
