"""Capture-grounded, lightly smoothed work centroid; no movement decisions here."""
from collections import deque
import numpy as np
from .detect_objects import Object
from .find import locate

# Average palm jitter over 0.6 s and publish once 0.4 s of continuous hands
# exist, so the runtime can follow work continuously like face gaze.
SMOOTH_NS = 600_000_000
READY_NS = 400_000_000


class HandWindow:
    def __init__(self):
        self.samples = deque(maxlen=32)
        self.last = None

    def clear(self):
        self.samples.clear()
        self.last = None

    def observe(self, palms, stamp, intrinsics, camera):
        if self.last is not None and not 0 < stamp - self.last <= 1_000_000_000:
            self.clear()
        self.last = stamp
        if not palms:
            self.samples.clear()
            return None
        centre = np.mean([palm.centre for palm in palms], axis=0)
        obj = Object('hand', [centre[0] - .5, centre[1] - .5, 1., 1.], 1.)
        point = locate([obj], intrinsics, camera, stamp)[0]['point']
        if point is None or point[1] >= 0 or np.linalg.norm(point[:2]) > 1.:
            self.samples.clear()
            return None
        self.samples.append((stamp, point))
        # Keep one boundary sample so a 5 Hz window spans the smoothing time
        # despite capture jitter. A gap above one second resets it.
        while len(self.samples) > 1 and self.samples[1][0] <= stamp - SMOOTH_NS:
            self.samples.popleft()
        if stamp - self.samples[0][0] < READY_NS:
            return None
        return np.mean([sample[1] for sample in self.samples], axis=0).tolist()
