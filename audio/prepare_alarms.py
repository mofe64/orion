"""Prepare bundled alarm PCM. Requires ffmpeg when authoring, never on the Pi."""
from array import array
from pathlib import Path
import subprocess
import sys


DIRECTORY = Path(__file__).resolve().parent / "alarms"
SOURCES = {
    "club_alarm": "soundreality-club-alarma-145490.mp3",
    "funny_alarm": "3dabrar-funny-alarm-317531.mp3",
}
RATE = 24000
PEAK = int(32767 * 0.65)


def prepare(name, source):
    raw = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", str(DIRECTORY / source),
         "-f", "s16le", "-ar", str(RATE), "-ac", "1", "pipe:1"],
        check=True, capture_output=True,
    ).stdout
    samples = array("h", raw)
    if sys.byteorder != "little":
        samples.byteswap()
    if not RATE <= len(samples) <= 30 * RATE:
        raise ValueError(f"{source}: expected between one and 30 seconds")
    peak = max(abs(value) for value in samples)
    if not peak:
        raise ValueError(f"{source}: audio is silent")
    gain = PEAK / peak
    fade = int(RATE * 0.012)
    for i, value in enumerate(samples):
        envelope = min(1, i / fade, (len(samples) - 1 - i) / fade)
        samples[i] = round(value * gain * envelope)
    if sys.byteorder != "little":
        samples.byteswap()
    (DIRECTORY / f"{name}.pcm").write_bytes(samples.tobytes())
    print(f"{name}: {len(samples) / RATE:.2f}s, 24 kHz mono PCM16, peak {PEAK}")


if __name__ == "__main__":
    for name, source in SOURCES.items():
        prepare(name, source)
