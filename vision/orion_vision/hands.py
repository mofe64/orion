"""Capture-grounded two-second work centroid; no movement decisions here."""
from collections import deque
import numpy as np
from .detect_objects import Object
from .find import locate


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
        # Keep one boundary sample so a 5 Hz window can prove the full two
        # seconds despite capture jitter. A gap above one second resets it.
        while len(self.samples) > 1 and self.samples[1][0] <= stamp - 2_000_000_000:
            self.samples.popleft()
        if stamp - self.samples[0][0] < 2_000_000_000:
            return None
        return np.mean([sample[1] for sample in self.samples], axis=0).tolist()
