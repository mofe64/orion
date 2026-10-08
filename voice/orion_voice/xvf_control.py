"""Read-only access to the XVF3800's USB vendor control interface.

The board answers control transfers beside its audio interfaces, so these
reads work while ALSA capture and playback are open. Each parameter lives at a
resource ID (wIndex) and command ID (wValue); a read sets bit 7 of the command
ID and the reply starts with one status byte. Values come from Seeed's
``python_control/xvf_host.py`` for firmware 2.0.x.
"""
from __future__ import annotations

from dataclasses import dataclass
import math
import struct
import time

VENDOR_ID = 0x2886
PRODUCT_ID = 0x001A
STATUS_OK = 0
STATUS_RETRY = 64
_READ_FLAG = 0x80
_FORMATS = {"uint8": ("B", 1), "uint16": ("H", 2), "int32": ("i", 4), "float": ("f", 4)}


@dataclass(frozen=True)
class Parameter:
    resource: int
    command: int
    count: int
    kind: str

    @property
    def reply_length(self) -> int:
        return 1 + self.count * _FORMATS[self.kind][1]


# Only parameters Orion reads. Azimuths are radians; beams are ordered
# focused 1, focused 2, free-running, auto-select.
PARAMETERS = {
    "VERSION": Parameter(48, 0, 3, "uint8"),
    "AEC_AECPATHCHANGE": Parameter(33, 0, 1, "int32"),
    "AEC_AECCONVERGED": Parameter(33, 3, 1, "int32"),
    "AEC_ASROUTONOFF": Parameter(33, 35, 1, "int32"),
    "AEC_ASROUTGAIN": Parameter(33, 36, 1, "float"),
    "AEC_MIC_ARRAY_GEO": Parameter(33, 74, 12, "float"),
    "AEC_AZIMUTH_VALUES": Parameter(33, 75, 4, "float"),
    "AEC_SPENERGY_VALUES": Parameter(33, 80, 4, "float"),
    "AUDIO_MGR_REF_GAIN": Parameter(35, 1, 1, "float"),
    "AUDIO_MGR_SELECTED_AZIMUTHS": Parameter(35, 11, 2, "float"),
    "AUDIO_MGR_OP_L": Parameter(35, 15, 2, "uint8"),
    "AUDIO_MGR_OP_R": Parameter(35, 19, 2, "uint8"),
    "AUDIO_MGR_SYS_DELAY": Parameter(35, 26, 1, "int32"),
    "DOA_VALUE": Parameter(20, 18, 2, "uint16"),
}

# Read once per measurement to record how the board is configured.
CONFIGURATION = ("VERSION", "AEC_ASROUTONOFF", "AEC_ASROUTGAIN", "AEC_MIC_ARRAY_GEO",
                 "AUDIO_MGR_REF_GAIN", "AUDIO_MGR_OP_L", "AUDIO_MGR_OP_R", "AUDIO_MGR_SYS_DELAY")
# Read on every poll.
LIVE = ("AEC_AZIMUTH_VALUES", "AEC_SPENERGY_VALUES", "AUDIO_MGR_SELECTED_AZIMUTHS",
        "AEC_AECCONVERGED", "AEC_AECPATHCHANGE", "DOA_VALUE")


def decode(name: str, reply: bytes) -> tuple:
    """Return the values in a successful reply; NaN floats become None."""
    parameter = PARAMETERS[name]
    if len(reply) != parameter.reply_length:
        raise ValueError(f"{name} reply has {len(reply)} bytes, expected {parameter.reply_length}")
    if reply[0] != STATUS_OK:
        raise ValueError(f"{name} reply has status {reply[0]}")
    code = _FORMATS[parameter.kind][0]
    values = struct.unpack("<" + code * parameter.count, bytes(reply[1:]))
    if parameter.kind == "float":
        return tuple(None if math.isnan(value) else value for value in values)
    return values


class XvfControl:
    """Read XVF3800 parameters through ``transfer(value, index, length) -> bytes``,
    a vendor IN control transfer on the device or a test double."""

    def __init__(self, transfer, *, retries: int = 100, sleep=time.sleep):
        self._transfer = transfer
        self._retries = retries
        self._sleep = sleep

    @classmethod
    def open(cls) -> "XvfControl":
        try:
            import usb.core
            import usb.util
        except ImportError as error:
            raise RuntimeError("pyusb is not installed in the voice environment") from error
        device = usb.core.find(idVendor=VENDOR_ID, idProduct=PRODUCT_ID)
        if device is None:
            raise RuntimeError("No XVF3800 found on USB (vendor 2886, product 001a)")
        request_type = usb.util.CTRL_IN | usb.util.CTRL_TYPE_VENDOR | usb.util.CTRL_RECIPIENT_DEVICE

        def transfer(value, index, length):
            try:
                return bytes(device.ctrl_transfer(request_type, 0, value, index, length, 1000))
            except usb.core.USBError as error:
                if error.errno == 13:
                    raise RuntimeError("Permission denied opening the XVF3800; install the "
                                       "udev rule or run as root") from error
                raise
        return cls(transfer)

    def read(self, name: str) -> tuple:
        parameter = PARAMETERS[name]
        for _ in range(self._retries):
            reply = self._transfer(_READ_FLAG | parameter.command, parameter.resource,
                                   parameter.reply_length)
            # The board answers RETRY while its servicer is still busy.
            if reply and reply[0] == STATUS_RETRY:
                self._sleep(0.001)
                continue
            return decode(name, reply)
        raise RuntimeError(f"{name} stayed busy after {self._retries} reads")

    def snapshot(self, names) -> dict:
        return {name: list(self.read(name)) for name in names}
