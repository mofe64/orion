"""ALSA microphone capture for the Pi listener."""

from __future__ import annotations

from contextlib import suppress
from pathlib import Path
import os
import select
import subprocess
import time


WAKE_SAMPLE_RATE = 16_000
WAKE_CHUNK_MILLISECONDS = 100
DEFAULT_CAPTURE_DEVICE = "plughw:CARD=seeed2micvoicec,DEV=0"
DEFAULT_CAPTURE_CARD = "seeed2micvoicec"
DEFAULT_CAPTURE_CONFIGURATOR = (
    Path(__file__).resolve().parents[2]
    / "hardware"
    / "audio"
    / "configure-capture.sh"
)
class AlsaPcmCapture:
    """Read transient mono PCM from Orion's stable ALSA capture device."""

    def __init__(
        self,
        device: str = DEFAULT_CAPTURE_DEVICE,
        *,
        card_name: str = DEFAULT_CAPTURE_CARD,
        configurator: Path = DEFAULT_CAPTURE_CONFIGURATOR,
    ) -> None:
        if not device.strip():
            raise ValueError("ALSA capture device cannot be empty")
        if not card_name.strip():
            raise ValueError("ALSA capture card cannot be empty")
        self._device = device
        self._card_name = card_name
        self._configurator = Path(configurator)
        self._process: subprocess.Popen[bytes] | None = None
        self._chunk_bytes = WAKE_SAMPLE_RATE * 2 * WAKE_CHUNK_MILLISECONDS // 1_000

    def configure_command(self) -> list[str]:
        return [str(self._configurator), self._card_name]

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
        if not self._configurator.is_file():
            raise FileNotFoundError(
                f"microphone configurator does not exist: {self._configurator}"
            )
        subprocess.run(
            self.configure_command(),
            check=True,
            stdout=subprocess.DEVNULL,
        )
        self._process = subprocess.Popen(self.command(), stdout=subprocess.PIPE)
        try:
            # ADC startup can reset the codec's mixer state. Apply the selected
            # gain after frames arrive, then discard the settling audio.
            self._discard_startup(300)
            subprocess.run(self.configure_command(), check=True, stdout=subprocess.DEVNULL)
            self._discard_startup(300)
        except BaseException:
            self.close()
            raise

    def _discard_startup(self, milliseconds):
        channels = int(self.command()[-1])
        remaining = WAKE_SAMPLE_RATE * 2 * channels * milliseconds // 1000
        fd = self._process.stdout.fileno()
        deadline = time.monotonic() + 3
        while remaining:
            timeout = deadline - time.monotonic()
            if timeout <= 0 or not select.select([fd], [], [], timeout)[0]:
                raise RuntimeError("ALSA microphone did not produce startup frames")
            chunk = os.read(fd, min(remaining, 8192))
            if not chunk: raise RuntimeError("ALSA microphone stopped during startup")
            remaining -= len(chunk)

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

