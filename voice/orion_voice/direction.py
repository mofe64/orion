"""Coarse stereo TDOA observations; calibration is explicit, not inferred."""
from collections import deque
import time
import numpy as np


class DirectionEstimator:
    def __init__(self, spacing_m: float = 0.0, channel_sign: int = 0, *, clock=time.monotonic):
        if not 0 <= spacing_m <= 0.3 or channel_sign not in {-1, 0, 1}:
            raise ValueError("Invalid microphone spacing or orientation")
        self.spacing_m = spacing_m
        self.channel_sign = channel_sign
        self.clock = clock
        self.votes = deque(maxlen=30)

    def reset(self):
        self.votes.clear()

    def _expire(self, now):
        while self.votes and now - self.votes[0][0] >= 3.0:
            self.votes.popleft()

    def accept(self, stereo: np.ndarray):
        now = self.clock()
        self._expire(now)
        if not self.spacing_m or not self.channel_sign:
            return
        x, y = stereo[:, 0].astype(float), stereo[:, 1].astype(float)
        x -= x.mean()
        y -= y.mean()
        if min(np.sqrt(np.mean(x*x)), np.sqrt(np.mean(y*y))) < 500:
            return
        if np.max(np.abs(stereo.astype(float))) > 32000:
            return
        n = 1 << (len(x) * 2 - 1).bit_length()
        cross = np.fft.rfft(x, n) * np.conj(np.fft.rfft(y, n))
        correlation = np.fft.irfft(cross / np.maximum(np.abs(cross), 1e-9), n)
        limit = max(1, int(np.ceil(self.spacing_m / 343.0 * 16000)))
        window = np.concatenate((correlation[-limit:], correlation[:limit+1]))
        peak = int(np.argmax(window))
        lag = peak - limit
        # Ambiguous peaks and physically implausible delays are not directions.
        competitors = window.copy()
        competitors[peak] = 0
        if window[peak] < 0.1 or window[peak] < 1.4 * np.max(competitors):
            return
        if abs(lag) > self.spacing_m / 343.0 * 16000 + 0.5:
            return
        self.votes.append((now, int(np.sign(lag)) * self.channel_sign))

    def observation(self):
        self._expire(self.clock())
        if len(self.votes) < 5:
            return {"side": "unknown", "confidence": 0.0, "observed_at": None}
        votes = [vote for _, vote in self.votes]
        winner = max((-1, 0, 1), key=votes.count)
        # Agreement is not a calibrated probability of identifying the speaker.
        confidence = votes.count(winner) / len(votes)
        side = {-1: "left", 0: "centre", 1: "right"}[winner]
        # Do not let a single new vote refresh older supporting evidence.
        observed_at = next(at for at, vote in self.votes if vote == winner)
        return {"side": side if confidence >= 0.75 else "unknown",
                "confidence": confidence, "observed_at": observed_at}
