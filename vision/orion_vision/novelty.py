"""Short-lived feature evidence after a classified object appears; no extra model.

Pixels and at most 24 feature points stay in this inference worker's memory.
A failed forward/backward flow check discards the candidate instead of guessing.
"""
import cv2
import numpy as np

from .behaviours import overlaps
from .detect_objects import Object
from .intrinsics import SIZE, lores_bgr


def gray(buffer):
    return cv2.cvtColor(lores_bgr(buffer), cv2.COLOR_BGR2GRAY)


class NoveltyTrack:
    def __init__(self, candidate, buffer, stamp):
        self.candidate = candidate
        self.box = list(candidate['box'])
        self.since = self.last = stamp
        self.previous = gray(buffer)
        mask = np.zeros(self.previous.shape, np.uint8)
        x, y, width, height = self.box
        left, top = max(0, int(x)), max(0, int(y))
        right, bottom = min(SIZE[0], int(x + width)), min(SIZE[1], int(y + height))
        mask[top:bottom, left:right] = 255
        self.points = cv2.goodFeaturesToTrack(self.previous, 24, .03, 4, mask=mask)
        if self.points is None or len(self.points) < 6:
            raise ValueError('Object has insufficient features to verify persistence')
        self.initial_count = len(self.points)

    def advance(self, buffer, stamp, faces):
        if not 0 < stamp - self.last <= 1_000_000_000 or stamp - self.since > 12_000_000_000:
            return None
        current = gray(buffer)
        moved, forward, _ = cv2.calcOpticalFlowPyrLK(self.previous, current, self.points, None,
            winSize=(15, 15), maxLevel=2, criteria=(cv2.TERM_CRITERIA_EPS | cv2.TERM_CRITERIA_COUNT, 15, .03))
        if moved is None:
            return None
        returned, backward, _ = cv2.calcOpticalFlowPyrLK(current, self.previous, moved, None,
            winSize=(15, 15), maxLevel=2, criteria=(cv2.TERM_CRITERIA_EPS | cv2.TERM_CRITERIA_COUNT, 15, .03))
        if returned is None:
            return None
        good = (forward.reshape(-1).astype(bool) & backward.reshape(-1).astype(bool)
                & (np.linalg.norm(returned - self.points, axis=2).reshape(-1) < 1.5)
                & np.isfinite(moved).all(axis=(1, 2)))
        if good.sum() < max(6, .6 * self.initial_count):
            return None
        delta = np.median(moved[good] - self.points[good], axis=0).reshape(2)
        if np.linalg.norm(delta) > 50:
            return None
        self.box[0] += float(delta[0]); self.box[1] += float(delta[1])
        x, y, width, height = self.box
        if (x < 0 or y < 0 or x + width > SIZE[0] or y + height > SIZE[1]
                or any(overlaps(self.box, face.box) for face in faces)):
            return None
        self.previous, self.points, self.last = current, moved[good], stamp
        return Object(self.candidate['category'], self.box.copy(), self.candidate['confidence'])
