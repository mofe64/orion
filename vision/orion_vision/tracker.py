"""Constant-velocity Kalman tracks in base bearings, never camera pixels."""
from dataclasses import dataclass
import math

import numpy as np

MAX_AGE_NS = 1_000_000_000
ASSOCIATION_RADIANS = math.radians(20)


def angle_delta(value):
    return (value + math.pi) % (2 * math.pi) - math.pi


@dataclass
class Track:
    id: int
    state: np.ndarray
    covariance: np.ndarray
    born_ns: int
    capture_ns: int
    face: object

    def observation(self, now_ns):
        x, y, width, height = self.face.box
        left, top = np.clip(x / 320, 0, 1), np.clip(y / 240, 0, 1)
        right, bottom = np.clip((x + width) / 320, 0, 1), np.clip((y + height) / 240, 0, 1)
        return {"id": self.id, "class": "face", "track_id": self.id, "capture_boottime_ns": self.capture_ns,
            "azimuth": float(angle_delta(self.state[0])), "elevation": float(self.state[1]),
            "age_ms": max(0, now_ns - self.capture_ns) / 1e6,
            "track_age_ms": max(0, self.capture_ns - self.born_ns) / 1e6,
            "confidence": self.face.confidence, "looking_at_orion": self.face.looking_at_orion,
            "box": [float(left), float(top), float(right - left), float(bottom - top)],
            "box_pixels": self.face.box, "landmarks": self.face.landmarks, "image_size": [320, 240]}


class Tracker:
    def __init__(self):
        self.tracks = []
        self.next_id = 1
        self.last_ns = 0

    def clear(self):
        self.tracks = []  # IDs never restart after privacy/reopen.
        self.last_ns = 0

    def update(self, stamp, faces, bearings):
        if stamp <= self.last_ns or len(faces) != len(bearings) or len(faces) > 16:
            raise ValueError("Invalid timestamp or face/bearing count")
        self.last_ns = stamp
        self.tracks = [track for track in self.tracks if stamp - track.capture_ns <= MAX_AGE_NS]
        predicted = []
        for track in self.tracks:
            dt = (stamp - track.capture_ns) / 1e9
            transition = np.eye(4)
            transition[0, 2] = transition[1, 3] = dt
            state = transition @ track.state
            covariance = transition @ track.covariance @ transition.T + np.diag([.001, .001, .04, .04]) * dt
            predicted.append((state, covariance))
        measurements = [np.array([bearing["azimuth"], bearing["elevation"]]) for bearing in bearings]
        if any(not np.isfinite(value).all() for value in measurements):
            raise ValueError("Invalid bearing")
        pairs = []
        for i, (state, _) in enumerate(predicted):
            for j, value in enumerate(measurements):
                residual = value - state[:2]
                residual[0] = angle_delta(residual[0])
                distance = np.linalg.norm(residual)
                if distance <= ASSOCIATION_RADIANS:
                    pairs.append((distance, i, j))
        used_tracks, used_faces = set(), set()
        for _, i, j in sorted(pairs):
            if i in used_tracks or j in used_faces:
                continue
            used_tracks.add(i); used_faces.add(j)
            state, covariance = predicted[i]
            residual = measurements[j] - state[:2]
            residual[0] = angle_delta(residual[0])
            noise = np.eye(2) * math.radians(1) ** 2
            gain = np.linalg.solve(covariance[:2, :2] + noise, covariance[:2, :]).T
            track = self.tracks[i]
            track.state = state + gain @ residual
            # Joseph form keeps the small covariance symmetric/positive.
            correction = np.eye(4)
            correction[:, :2] -= gain
            track.covariance = correction @ covariance @ correction.T + gain @ noise @ gain.T
            track.capture_ns, track.face = stamp, faces[j]
        for j, value in enumerate(measurements):
            if j not in used_faces and len(self.tracks) < 16:
                self.tracks.append(Track(self.next_id, np.r_[value, 0., 0.],
                    np.diag([.01, .01, .5, .5]), stamp, stamp, faces[j]))
                self.next_id += 1
        return [track for track in self.tracks if track.capture_ns == stamp]

    def observations(self, now_ns):
        return [track.observation(now_ns) for track in self.tracks
            if 0 <= now_ns - track.capture_ns <= MAX_AGE_NS]


def choose_track(tracks):
    looking = [track for track in tracks if track.face.looking_at_orion]
    return max(looking or tracks, key=lambda track: track.face.box[2] * track.face.box[3], default=None)
