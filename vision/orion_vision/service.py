"""One latest-frame slot, a privacy watcher, and bounded Unix HTTP clients."""
import argparse
from contextlib import contextmanager
import json
import logging
import os
from pathlib import Path
import signal
import socket
import socketserver
import stat
from http.server import BaseHTTPRequestHandler
from threading import Condition, Event, Lock, Semaphore, Thread
import time
from urllib.parse import parse_qs, urlsplit

import numpy as np
import simplejpeg

from .sources import Picamera2Source, ReplaySource

LOG = logging.getLogger(__name__)
BOUNDARY = "orion-frame"


def camera_enabled(path):
    try:
        value = json.loads(Path(path).read_text())["camera_enabled"]
        return value if type(value) is bool else False
    except FileNotFoundError:
        return True
    except (OSError, ValueError, KeyError, TypeError):
        # Corrupt or unreadable preferences fail closed.
        return False


def exposure_midpoint(metadata):
    timestamp, exposure = metadata.get("SensorTimestamp"), metadata.get("ExposureTime")
    if type(timestamp) is not int or timestamp <= 0 or type(exposure) is not int or exposure < 0:
        return None
    return timestamp + exposure * 1000 // 2


class Vision:
    def __init__(self, source, preference_file, encoder=simplejpeg.encode_jpeg, clock=time.monotonic,
                 pipeline=None, detection_error=None):
        self.source, self.preference_file, self.encoder = source, Path(preference_file), encoder
        self.clock = clock
        self.pipeline, self.detection_error = pipeline, detection_error
        self.changed = Condition()
        self.encode_lock = Lock()
        self.stop = Event()
        self.enabled = False
        self.error = None
        self.latest = None
        self.cached = None
        self.last_encode = 0.
        self.last_capture = 0
        self.last_sensor = 0
        self.generation = 0
        self.dropped = 0
        self.encodes = 0
        self.clients = 0
        self.snapshot_until = 0.
        self.thread = None

    def start(self):
        if self.pipeline:
            self.pipeline.start()
        self.thread = Thread(target=self._watch, name="vision-privacy", daemon=True)
        self.thread.start()

    def close(self):
        self.stop.set()
        with self.changed:
            self.changed.notify_all()
        if self.thread is not None:
            self.thread.join(timeout=2)
        if self.pipeline:
            self.pipeline.close()

    def _disable(self):
        with self.changed:
            self.enabled = False
            self.generation += 1
            self.latest = self.cached = None
            self.last_capture = self.last_sensor = 0
            self.snapshot_until = 0.
            self.changed.notify_all()
        self.source.close()
        if self.pipeline:
            self.pipeline.reset(False)

    def _watch(self):
        retry_at = 0.
        try:
            while not self.stop.is_set():
                wanted = camera_enabled(self.preference_file)
                if not wanted and self.enabled:
                    self._disable()
                elif wanted and not self.enabled and time.monotonic() >= retry_at:
                    with self.changed:
                        self.enabled = True
                        self.error = None
                        generation = self.generation
                    try:
                        if self.pipeline:
                            self.pipeline.reset(True)
                            self.source.start(lambda capture: self.publish(capture, generation), self.wants_rgb,
                                              lambda: self.enabled)
                        else:
                            self.source.start(lambda capture: self.publish(capture, generation), self.wants_rgb)
                    except Exception as error:
                        LOG.exception("Camera start failed")
                        self._disable()
                        self.error = str(error)
                        retry_at = time.monotonic() + 1
                self.stop.wait(.1)
        finally:
            self._disable()

    def publish(self, capture, generation):
        stamp = exposure_midpoint(capture.metadata)
        sensor = capture.metadata.get("SensorTimestamp")
        with self.changed:
            if not self.enabled or generation != self.generation:
                return
            if stamp is None or sensor <= self.last_sensor or stamp <= self.last_capture:
                self.dropped += 1
                return
            if capture.rgb is not None and (capture.rgb.dtype != np.uint8 or capture.rgb.ndim != 3 or capture.rgb.shape[2] != 3):
                self.dropped += 1
                return
            self.last_sensor, self.last_capture = sensor, stamp
            self.latest = (stamp, capture.rgb) if capture.rgb is not None else None
            if capture.rgb is None:
                self.cached = None
            self.changed.notify_all()
            if self.pipeline:
                self.pipeline.submit(stamp, capture.lores, capture.metadata, capture.geometry)

    def wants_rgb(self):
        # This callback does no I/O or encoding. Stream admission happens
        # before waiting for its first frame, so idle capture can serve it.
        with self.changed:
            return self.enabled and (self.clients > 0 or self.clock() < self.snapshot_until)

    @contextmanager
    def viewing(self, snapshot=False):
        with self.changed:
            if snapshot:
                self.snapshot_until = self.clock() + 2
            else:
                self.clients += 1
        try:
            yield
        finally:
            if not snapshot:
                with self.changed:
                    self.clients -= 1

    def wait_for_jpeg(self):
        with self.changed:
            self.changed.wait_for(lambda: self.latest is not None or not self.enabled, timeout=1)
        return self.jpeg()

    def status(self):
        with self.changed:
            if not camera_enabled(self.preference_file):
                return 503, {"error": {"code": "camera_off", "message": "Camera is disabled"}}
            if not self.enabled or self.error:
                return 503, {"error": {"code": "camera_unavailable", "message": self.error or "Camera is starting"}}
            return 200, {"camera_enabled": True, "capture_boottime_ns": self.last_capture or None,
                         "clock": "CLOCK_BOOTTIME", "stream_clients": self.clients, "dropped_frames": self.dropped,
                         "detection": self.pipeline.status() if self.pipeline else {"error": self.detection_error}}

    def jpeg(self, after=0):
        # Shared cache and throttle: ten viewers do not cause ten encodes.
        with self.encode_lock:
            with self.changed:
                if self.status()[0] != 200 or self.latest is None:
                    return None
                stamp, rgb = self.latest
                if self.cached and (stamp == self.cached[0] or time.monotonic() - self.last_encode < 1 / 15):
                    return self.cached if self.cached[0] > after else None
                if stamp <= after:
                    return None
                generation = self.generation
            height, width = rgb.shape[:2]
            # Bounded nearest-neighbour sampling needs no additional image library.
            resized = np.ascontiguousarray(rgb[np.arange(360) * height // 360][:, np.arange(640) * width // 640])
            jpeg = self.encoder(resized, quality=85, colorspace="RGB", colorsubsampling="420")
            with self.changed:
                if self.status()[0] != 200 or generation != self.generation:
                    return None
                self.cached = (stamp, jpeg)
                self.last_encode = time.monotonic()
                self.encodes += 1
                return self.cached


class VisionServer(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True

    def __init__(self, path, vision):
        self.vision = vision
        self.slots = Semaphore(16)
        super().__init__(str(path), Handler)
        os.chmod(path, 0o600)

    def process_request(self, request, address):
        if not self.slots.acquire(blocking=False):
            self.shutdown_request(request)
            return
        try:
            super().process_request(request, address)
        except BaseException:
            self.slots.release()
            raise

    def process_request_thread(self, request, address):
        try:
            super().process_request_thread(request, address)
        finally:
            self.slots.release()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"

    def setup(self):
        super().setup()
        self.connection.settimeout(1)

    def log_message(self, format, *args):
        pass

    def respond(self, status, body, content_type="application/json"):
        if content_type == "application/json":
            body = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        vision = self.server.vision
        try:
            status, body = vision.status()
            if status != 200:
                return self.respond(status, body)
            if self.path == "/health":
                return self.respond(200, body)
            if self.path == "/observations":
                return self.respond(200, vision.pipeline.observations() if vision.pipeline else [])
            if urlsplit(self.path).path == "/speaker":
                if not vision.pipeline:
                    return self.respond(503, {"error": {"code": "detection_unavailable"}})
                try:
                    query = parse_qs(urlsplit(self.path).query, strict_parsing=True)
                    if set(query) != {"since_ns"} or len(query["since_ns"]) != 1:
                        raise ValueError("Expected one since_ns timestamp")
                    since_ns = int(query["since_ns"][0])
                    result = vision.pipeline.speaker(since_ns)
                except (ValueError, KeyError) as error:
                    return self.respond(400, {"error": {"code": "invalid_window", "message": str(error)}})
                return self.respond(200, result)
            if self.path not in ("/frame.jpg", "/stream"):
                return self.respond(404, {"error": {"code": "not_found"}})
            with vision.viewing(snapshot=self.path == "/frame.jpg"):
                frame = vision.wait_for_jpeg()
                if frame is None:
                    status, body = vision.status()
                    return self.respond(status if status != 200 else 503,
                        body if status != 200 else {"error": {"code": "no_frame", "message": "Waiting for capture"}})
                if self.path == "/frame.jpg":
                    self.send_response(200)
                    self.send_header("Content-Type", "image/jpeg")
                    self.send_header("Content-Length", str(len(frame[1])))
                    self.send_header("X-Capture-Boottime-Ns", str(frame[0]))
                    self.send_header("Cache-Control", "no-store")
                    self.end_headers()
                    self.wfile.write(frame[1])
                    return
                self.send_response(200)
                self.send_header("Content-Type", f"multipart/x-mixed-replace; boundary={BOUNDARY}")
                self.send_header("Cache-Control", "no-store")
                self.end_headers()
                last = 0
                while not vision.stop.is_set() and vision.status()[0] == 200:
                    # Detect a departing peer before doing another encode.
                    import select
                    if select.select([self.connection], [], [], 0)[0] and not self.connection.recv(1, socket.MSG_PEEK):
                        break
                    if frame is not None and frame[0] > last:
                        stamp, jpeg = frame
                        self.wfile.write((f"--{BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {len(jpeg)}\r\n"
                                          f"X-Capture-Boottime-Ns: {stamp}\r\n\r\n").encode() + jpeg + b"\r\n")
                        self.wfile.flush()
                        last = stamp
                    with vision.changed:
                        vision.changed.wait(timeout=1 / 15)
                    frame = vision.jpeg(after=last)
        except (BrokenPipeError, ConnectionResetError, TimeoutError, OSError):
            pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--socket", type=Path, default=Path("/tmp/orion-vision.sock"))
    parser.add_argument("--preference-file", type=Path, default=Path.home() / ".config/orion/camera.json")
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--replay", type=Path, help="Directory of synthetic JPEG test inputs")
    from .calibration import DEFAULT_PATH, Intrinsics
    from .detect_faces import MODEL_PATH, YuNet
    from .gaze_client import GazeClient
    from .pipeline import FacePipeline
    parser.add_argument("--intrinsics", type=Path, default=DEFAULT_PATH)
    parser.add_argument("--face-model", type=Path, default=MODEL_PATH)
    parser.add_argument("--runtime-socket", type=Path, default=Path("/tmp/oriond.sock"))
    args = parser.parse_args()
    camera = json.loads(args.profile.read_text()).get("camera", {})
    if camera.get("enabled") is not True:
        parser.error("The hardware profile has no enabled camera")
    if camera.get('hflip') or camera.get('vflip'):
        parser.error("Fisheye rays require a profile with both flips false")
    source = ReplaySource(args.replay) if args.replay else Picamera2Source(
        camera["index"], camera["hflip"], camera["vflip"])
    logging.basicConfig(level=logging.INFO)
    if args.socket.exists():
        if not stat.S_ISSOCK(args.socket.stat().st_mode) or args.socket.stat().st_uid != os.getuid():
            raise RuntimeError("Refusing to replace an unowned or non-socket path")
        # Do not steal a socket from another running owner.
        with socket.socket(socket.AF_UNIX) as probe:
            try:
                probe.connect(str(args.socket))
            except ConnectionRefusedError:
                args.socket.unlink()
            else:
                raise RuntimeError("Vision socket already has a running owner")
    pipeline, detection_error = None, None
    try:
        pipeline = FacePipeline(YuNet(args.face_model), Intrinsics.load(args.intrinsics),
                                GazeClient(args.runtime_socket))
        if isinstance(source, Picamera2Source):
            source.expected_geometry = pipeline.intrinsics.geometry
    except (OSError, ValueError, KeyError) as error:
        detection_error = str(error)
        LOG.warning("Face tracking unavailable: %s", error)
    vision = Vision(source, args.preference_file, pipeline=pipeline, detection_error=detection_error)
    server = VisionServer(args.socket, vision)
    def stop(signum, frame):
        vision.stop.set()
        Thread(target=server.shutdown, daemon=True).start()
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    vision.start()
    try:
        server.serve_forever(poll_interval=.1)
    finally:
        vision.close()
        server.server_close()
        args.socket.unlink(missing_ok=True)


if __name__ == "__main__":
    main()
