"""Coarse stereo TDOA observations; calibration is explicit, not inferred."""
from collections import deque
import logging
import math
import threading
import time
import numpy as np

from .xvf_control import XvfControl


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


class XvfDirectionEstimator:
    """Calibrated beam-3 observations; only the poller touches USB.

    Positive lamp angles are counter-clockwise from the front (lamp left).
    Missing calibration or a USB failure disables observations for this run.
    """
    def __init__(self, offset_deg=None, sign=None, front_half_width=30.0, min_energy=0.0,
                 *, clock=time.monotonic, open_control=XvfControl.open, start_polling=True):
        self.clock = clock
        self.offset_deg, self.sign = offset_deg, sign
        self.front_half_width, self.min_energy = front_half_width, min_energy
        self.samples = deque(maxlen=64)
        self._lock = threading.Lock()
        self._stop = threading.Event()
        self._generation = 0
        self._failed = False
        self._thread = None
        self._calibrated = offset_deg is not None and sign is not None
        if not self._calibrated:
            logging.warning("XVF direction disabled: supply measured azimuth offset and sign")
            return
        if (sign not in (-1, 1) or not math.isfinite(offset_deg)
                or not 0 <= front_half_width <= 150 or not math.isfinite(min_energy)
                or min_energy < 0):
            raise ValueError("Invalid XVF direction calibration or energy threshold")
        if start_polling:
            self._thread = threading.Thread(target=self._poll, args=(open_control,),
                                            name="xvf-direction", daemon=True)
            self._thread.start()

    def _disable(self, error):
        with self._lock:
            self.samples.clear()
            self._failed = True
        logging.warning("XVF direction disabled: %s", error)

    def _poll(self, open_control):
        try:
            control = open_control()
            while not self._stop.is_set():
                started = self.clock()
                if not self._poll_once(control):
                    return
                self._stop.wait(max(0.0, 0.05 - (self.clock() - started)))
        except Exception as error:
            self._disable(error)

    def _poll_once(self, control):
        with self._lock:
            if self._failed or not self._calibrated:
                return False
            generation = self._generation
        at = self.clock()
        try:
            azimuth = control.read("AEC_AZIMUTH_VALUES")[3]
            energy = control.read("AEC_SPENERGY_VALUES")[3]
        except Exception as error:
            self._disable(error)
            return False
        with self._lock:
            # A reset can happen while USB is busy; don't resurrect old evidence.
            if generation == self._generation and not self._stop.is_set():
                self.samples.append((at, azimuth, energy))
        return True

    def close(self):
        self._stop.set()

    def reset(self):
        with self._lock:
            self._generation += 1
            self.samples.clear()

    def _expire(self, now):
        while self.samples and now - self.samples[0][0] >= 3.0:
            self.samples.popleft()

    def accept(self, stereo):
        with self._lock:
            self._expire(self.clock())

    def observation(self):
        with self._lock:
            self._expire(self.clock())
            samples = list(self.samples)
        votes = []
        for at, azimuth, energy in samples:
            if (azimuth is None or energy is None or not math.isfinite(azimuth)
                    or not math.isfinite(energy) or energy <= self.min_energy):
                continue
            lamp = (self.sign * (math.degrees(azimuth) - self.offset_deg) + 180) % 360 - 180
            side = "centre" if abs(lamp) <= self.front_half_width else "left" if lamp > 0 else "right"
            votes.append((at, side))
        if len(votes) < 5:
            return {"side": "unknown", "confidence": 0.0, "observed_at": None}
        sides = [side for _, side in votes]
        winner = max(("centre", "left", "right"), key=sides.count)
        confidence = sides.count(winner) / len(votes)
        return {"side": winner if confidence >= 0.75 else "unknown", "confidence": confidence,
                "observed_at": next(at for at, side in votes if side == winner)}
