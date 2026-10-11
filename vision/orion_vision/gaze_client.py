"""Bounded Unix requests. Vision sends bearings; only oriond owns kinematics."""
import json
import socket
import time


def boottime_ns():
    if hasattr(time, "CLOCK_BOOTTIME"):
        return time.clock_gettime_ns(time.CLOCK_BOOTTIME)
    # Workstation replay uses the same continuous clock as the Rust daemon.
    import ctypes
    class Timebase(ctypes.Structure):
        _fields_ = [("numer", ctypes.c_uint32), ("denom", ctypes.c_uint32)]
    library = ctypes.CDLL("/usr/lib/libSystem.B.dylib")
    library.mach_continuous_time.restype = ctypes.c_uint64
    scale = Timebase()
    library.mach_timebase_info(ctypes.byref(scale))
    return library.mach_continuous_time() * scale.numer // scale.denom


class RuntimeRejected(ValueError):
    """A valid runtime reply explicitly refused this command."""


class GazeClient:
    def __init__(self, path="/tmp/oriond.sock", timeout=.2):
        self.path, self.timeout = str(path), timeout

    def command(self, command):
        deadline = time.monotonic() + self.timeout
        with socket.socket(socket.AF_UNIX) as client:
            client.settimeout(self.timeout)
            client.connect(self.path)
            client.sendall(command.encode() + b"\n")
            data = bytearray()
            while b"\n" not in data:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError("Runtime request deadline exceeded")
                client.settimeout(remaining)
                chunk = client.recv(4096)
                if not chunk or len(data) + len(chunk) > 65536:
                    raise ValueError("Missing or oversized runtime response")
                data.extend(chunk)
        value = json.loads(data.split(b"\n", 1)[0])
        if value.get("ok") is not True:
            raise RuntimeRejected(value.get("error", "Runtime rejected request"))
        return value

    def bearings(self, stamp, rays):
        value = self.command("camera-rays " + json.dumps({"capture_boottime_ns": stamp, "rays": rays.tolist()}))
        if len(value["bearings"]) != len(rays):
            raise ValueError("Runtime bearing count mismatch")
        return value["bearings"]

    def camera_pose(self, stamp):
        return self.command('camera-rays ' + json.dumps({
            'capture_boottime_ns': stamp, 'rays': [[0, 0, 1]]}))['camera']

    def find_allowed(self, session=None):
        value = self.command('voice status')
        voice = value['voice']
        if session is not None:
            return value.get('vision_session') == session
        # Explicit non-voice callers and future background work fail closed
        # for every live phase, including wake verification and announcements.
        return not voice.get('session')

    def target(self, track):
        from .tracker import angle_delta
        try:
            self.command("gaze target " + json.dumps({"track_id": track.id,
                "capture_boottime_ns": track.capture_ns, "azimuth": float(angle_delta(track.state[0])),
                "elevation": float(track.state[1]), "confidence": track.face.confidence, "class": "face"}))
        except RuntimeRejected:
            return False
        return True

    def gaze_available(self):
        return self.command("gaze status")["gaze"]["enabled"] is True

    def clear(self):
        self.command("gaze clear")

    def voice_active(self):
        voice = self.command("voice status")["voice"]
        return bool(voice.get("session") and voice.get("confirmed")
            and voice.get("phase") not in ("guard", "window", "unavailable", ""))

    def conversation_active(self):
        voice = self.command("voice status")["voice"]
        return bool(voice.get("session") and voice.get("confirmed")
                    and voice.get("phase") not in ("unavailable", ""))
