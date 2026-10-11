"""Constant-velocity Kalman tracks in base bearings, never camera pixels."""
from collections import deque
from dataclasses import dataclass, field
import math

import numpy as np

from .intrinsics import SIZE

MAX_AGE_NS = 1_000_000_000
ASSOCIATION_RADIANS = math.radians(20)
MOUTH_MOTION_MIN = .05
MOUTH_WINNER_MARGIN = .05


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
    mouth_history: deque = field(default_factory=lambda: deque(maxlen=32))

    def record_mouth(self):
        landmarks = np.asarray(self.face.landmarks, dtype=float)
        axis = landmarks[1] - landmarks[0]
        span = np.linalg.norm(axis)
        if span < 1:
            return
        axis /= span
        perpendicular = np.array([-axis[1], axis[0]])
        relative = (landmarks[3:] - landmarks[:2].mean(axis=0)) / span
        signature = np.column_stack((relative @ axis, relative @ perpendicular)).ravel()
        self.mouth_history.append((self.capture_ns, signature))

    def mouth_motion(self, since_ns, now_ns):
        history = [(stamp, value) for stamp, value in self.mouth_history if stamp <= now_ns]
        window = [value for stamp, value in history if stamp >= since_ns]
        if len(window) < 2:
            return 0.
        baseline = [value for stamp, value in history if stamp < since_ns]
        # Median adjacent movement resists isolated landmark outliers. Estimate
        # each face's own jitter before the requested window; short histories
        # use a conservative floor. Evidence still depends on sample count/rate.
        noise = max(.05, float(np.percentile(np.linalg.norm(
            np.diff(baseline, axis=0), axis=1), 75))) if len(baseline) >= 4 else .2
        movement = float(np.median(np.linalg.norm(np.diff(window, axis=0), axis=1)))
        return max(0., movement - noise)

    def observation(self, now_ns):
        x, y, width, height = self.face.box
        left, top = np.clip(x / SIZE[0], 0, 1), np.clip(y / SIZE[1], 0, 1)
        right, bottom = np.clip((x + width) / SIZE[0], 0, 1), np.clip((y + height) / SIZE[1], 0, 1)
        return {"id": self.id, "class": "face", "track_id": self.id, "capture_boottime_ns": self.capture_ns,
            "azimuth": float(angle_delta(self.state[0])), "elevation": float(self.state[1]),
            "age_ms": max(0, now_ns - self.capture_ns) / 1e6,
            "track_age_ms": max(0, self.capture_ns - self.born_ns) / 1e6,
            "confidence": self.face.confidence, "looking_at_orion": self.face.looking_at_orion,
            "box": [float(left), float(top), float(right - left), float(bottom - top)],
            "box_pixels": self.face.box, "landmarks": self.face.landmarks, "image_size": list(SIZE)}


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
            track.record_mouth()
        for j, value in enumerate(measurements):
            if j not in used_faces and len(self.tracks) < 16:
                self.tracks.append(Track(self.next_id, np.r_[value, 0., 0.],
                    np.diag([.01, .01, .5, .5]), stamp, stamp, faces[j]))
                self.tracks[-1].record_mouth()
                self.next_id += 1
        return [track for track in self.tracks if track.capture_ns == stamp]

    def observations(self, now_ns):
        return [track.observation(now_ns) for track in self.tracks
            if 0 <= now_ns - track.capture_ns <= MAX_AGE_NS]

    def speaker(self, since_ns, now_ns):
        if type(since_ns) is not int or since_ns < 0 or since_ns > now_ns + 50_000_000:
            raise ValueError("Invalid speaker window timestamp")
        since_ns = max(since_ns, now_ns - 3_000_000_000)
        recent = [track for track in self.tracks if 0 <= now_ns - track.capture_ns <= MAX_AGE_NS]
        ranked = sorted(((track.mouth_motion(since_ns, now_ns), track) for track in recent),
            key=lambda item: item[0], reverse=True)
        score, moving = ranked[0] if ranked else (0., None)
        runner_up = ranked[1][0] if len(ranked) > 1 else 0.
        if (moving and score >= MOUTH_MOTION_MIN and score >= runner_up + MOUTH_WINNER_MARGIN
                and score >= 1.5 * runner_up):
            chosen, strategy = moving, "mouth_motion"
        else:
            frontal = [track for track in recent if track.face.looking_at_orion]
            chosen = choose_track(frontal or recent)
            strategy = "looking_at_orion" if frontal else "nearest" if chosen else "none"
        return {"speaker": chosen.observation(now_ns) if chosen else None,
            "strategy": strategy, "since_boottime_ns": since_ns,
            "observed_boottime_ns": self.last_ns or None}


def choose_track(tracks):
    looking = [track for track in tracks if track.face.looking_at_orion]
    return max(looking or tracks, key=lambda track: track.face.box[2] * track.face.box[3], default=None)
