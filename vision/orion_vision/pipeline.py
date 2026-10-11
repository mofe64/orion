"""One inference worker and one replaceable frame slot; no callback inference."""
from threading import Condition, Event, Lock, Thread
import time

from .gaze_client import boottime_ns
from .tracker import Tracker, choose_track

IDLE_HZ, VOICE_HZ = 10, 3


class FacePipeline:
    def __init__(self, detector, intrinsics, client, clock=time.monotonic, boot_clock=boottime_ns):
        self.detector, self.intrinsics, self.client = detector, intrinsics, client
        self.clock, self.boot_clock = clock, boot_clock
        self.changed, self.stop = Condition(), Event()
        self.commands = Lock()
        self.pending = None
        self.enabled = False
        self.generation = 0
        self.dropped = self.inferences = 0
        self.tracker = Tracker()
        self.error = None
        self.rate_hz = IDLE_HZ
        self.next_voice_poll = 0.
        self.thread = None

    def start(self):
        self.thread = Thread(target=self._run, name="vision-inference", daemon=True)
        self.thread.start()

    def reset(self, enabled):
        with self.changed:
            self.enabled = enabled
            self.generation += 1
            self.pending = None
            self.tracker.clear()
            self.changed.notify_all()
        if not enabled:
            with self.commands:
                try:
                    self.client.clear()
                except (OSError, ValueError):
                    pass

    def close(self):
        self.reset(False)
        self.stop.set()
        with self.changed:
            self.changed.notify_all()
        if self.thread:
            self.thread.join(timeout=2)

    def submit(self, stamp, buffer):
        with self.changed:
            if self.enabled and buffer is not None:
                self.dropped += self.pending is not None
                self.pending = (stamp, buffer, self.generation)
                self.changed.notify()

    def observations(self):
        with self.changed:
            return self.tracker.observations(self.boot_clock()) if self.enabled else []

    def status(self):
        with self.changed:
            return {"rate_hz": self.rate_hz, "inferences": self.inferences,
                "dropped_frames": self.dropped, "error": self.error}

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
                stamp, buffer, generation = self.pending
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
                        continue
                    tracks = self.tracker.update(stamp, faces, bearings)
                    chosen = choose_track(tracks)
                with self.commands:
                    with self.changed:
                        if not self.enabled or generation != self.generation:
                            continue
                    if chosen:
                        self.client.target(chosen)
                    else:
                        self.client.clear()
                with self.changed:
                    self.error = None
            except Exception as error:
                with self.changed:
                    self.error = str(error)
                    self.tracker.clear()
                with self.commands:
                    try:
                        self.client.clear()
                    except (OSError, ValueError):
                        pass
