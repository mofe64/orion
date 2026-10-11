"""One inference worker and one replaceable frame slot; no callback inference."""
from threading import Condition, Event, Lock, Thread
import time

from .gaze_client import boottime_ns
from .tracker import MAX_AGE_NS, Tracker, choose_track

IDLE_HZ, VOICE_HZ = 10, 3
GAZE_RETRY_SECONDS = 3.


class FacePipeline:
    def __init__(self, detector, intrinsics, client, clock=time.monotonic, boot_clock=boottime_ns):
        self.detector, self.intrinsics, self.client = detector, intrinsics, client
        self.clock, self.boot_clock = clock, boot_clock
        self.changed, self.stop = Condition(), Event()
        self.commands = Lock()
        self.pending = None
        self.latest_preview = None
        self.preview_generation = 0
        self.enabled = False
        self.generation = 0
        self.dropped = self.inferences = 0
        self.tracker = Tracker()
        self.error = None
        self.rate_hz = IDLE_HZ
        self.next_voice_poll = 0.
        self.thread = None
        self.gaze_available = True
        self.next_gaze_poll = 0.
        self.gaze_error = None
        self.target_selected = False
        self.last_gaze_capture = 0

    def start(self):
        self.thread = Thread(target=self._run, name="vision-inference", daemon=True)
        self.thread.start()

    def reset(self, enabled):
        with self.changed:
            self.enabled = enabled
            self.generation += 1
            self.pending = None
            self.latest_preview = None
            self.preview_generation += 1
            self.tracker.clear()
            self.changed.notify_all()
        if not enabled:
            with self.commands:
                self._clear_target()

    def close(self):
        self.reset(False)
        self.stop.set()
        with self.changed:
            self.changed.notify_all()
        if self.thread:
            self.thread.join(timeout=2)

    def submit(self, stamp, buffer, metadata=None, geometry=None, rgb=None):
        with self.changed:
            if self.enabled and buffer is not None:
                if self.intrinsics.geometry is not None:
                    try:
                        self.intrinsics.validate_capture(buffer, metadata, geometry)
                    except ValueError as error:
                        self.dropped += 1
                        self.error = str(error)
                        self.clear_preview()
                        return
                self.dropped += self.pending is not None
                self.pending = (stamp, buffer, self.generation, rgb, self.preview_generation)
                self.changed.notify()

    def preview(self):
        with self.changed:
            return self.latest_preview if self.enabled else None

    def clear_preview(self):
        # Viewer departure invalidates RGB in flight without resetting gaze tracks.
        with self.changed:
            self.latest_preview = None
            self.preview_generation += 1
            if self.pending is not None:
                stamp, buffer, generation, _, _ = self.pending
                self.pending = (stamp, buffer, generation, None, self.preview_generation)

    def observations(self):
        with self.changed:
            return self.tracker.observations(self.boot_clock()) if self.enabled else []

    def status(self):
        with self.changed:
            return {"rate_hz": self.rate_hz, "inferences": self.inferences,
                "dropped_frames": self.dropped, "error": self.error,
                "gaze_available": self.gaze_available, "gaze_error": self.gaze_error}

    def _clear_target(self):
        if self.target_selected:
            self.target_selected = False
            try:
                self.client.clear()
            except (OSError, ValueError):
                pass

    def _send_gaze(self, chosen):
        now = self.clock()
        try:
            if chosen is None:
                self._clear_target()
                return
            if not self.gaze_available and now >= self.next_gaze_poll:
                self.next_gaze_poll = now + GAZE_RETRY_SECONDS
                self.gaze_available = self.client.gaze_available()
            if not self.gaze_available:
                return
            if chosen:
                if chosen.capture_ns <= self.last_gaze_capture:
                    return  # A retained track is not a fresh camera measurement.
                if self.client.target(chosen) is False:
                    self.gaze_available = False
                    self.next_gaze_poll = now + GAZE_RETRY_SECONDS
                    self.gaze_error = "Runtime gaze is unavailable"
                else:
                    self.target_selected = True
                    self.last_gaze_capture = chosen.capture_ns
                    self.gaze_error = None
        except (OSError, ValueError, KeyError) as error:
            # Motion availability does not invalidate detector/ray evidence.
            self.gaze_available = False
            self.next_gaze_poll = now + GAZE_RETRY_SECONDS
            self.gaze_error = str(error)

    def speaker(self, since_ns):
        with self.changed:
            return self.tracker.speaker(since_ns, self.boot_clock())

    def _run(self):
        next_inference = 0.
        last_inference = None
        while not self.stop.is_set():
            with self.changed:
                self.changed.wait_for(lambda: self.stop.is_set() or self.pending is not None, timeout=.1)
                if self.stop.is_set():
                    return
                if self.pending is None:
                    continue
            now = self.clock()
            if now >= self.next_voice_poll:
                try:
                    active = self.client.voice_active()
                except (OSError, ValueError):
                    active = True  # Reduced CPU use while voice state is unknown.
                self.rate_hz = VOICE_HZ if active else IDLE_HZ
                if active and last_inference is not None:
                    next_inference = max(next_inference, last_inference + 1 / VOICE_HZ)
                self.next_voice_poll = now + .2
            if now < next_inference:
                self.stop.wait(min(next_inference - now, .05))
                continue
            with self.changed:
                if not self.enabled or self.pending is None:
                    continue
                stamp, buffer, generation, rgb, preview_generation = self.pending
                self.pending = None
            last_inference = self.clock()
            next_inference = last_inference + 1 / self.rate_hz
            try:
                faces = self.detector.detect(buffer)
                bearings = self.client.bearings(stamp, self.intrinsics.rays([face.centre for face in faces])) if faces else []
                with self.changed:
                    self.inferences += 1
                    if not self.enabled or generation != self.generation:
                        continue
                    # Drop late results instead of sending a stale target.
                    if not 0 <= self.boot_clock() - stamp <= 1_000_000_000:
                        self.error = "Detection result is stale"
                        self.latest_preview = None
                        continue
                    fresh = self.tracker.update(stamp, faces, bearings)
                    tracks = [track for track in self.tracker.tracks
                        if 0 <= self.boot_clock() - track.capture_ns <= MAX_AGE_NS]
                    chosen = choose_track(tracks)
                    if rgb is not None and preview_generation == self.preview_generation:
                        detections = [{"box": track.observation(stamp)["box"],
                            "score": track.face.confidence, "track_id": track.id,
                            "chosen_target": track is chosen, "capture_boottime_ns": str(stamp)}
                            for track in fresh]
                        self.latest_preview = (stamp, rgb, detections)
                    else:
                        self.latest_preview = None
                with self.commands:
                    with self.changed:
                        if not self.enabled or generation != self.generation:
                            continue
                    self._send_gaze(chosen)
                with self.changed:
                    self.error = None
            except Exception as error:
                with self.changed:
                    self.error = str(error)
                    self.tracker.clear()
                    self.latest_preview = None
                with self.commands:
                    self._clear_target()
