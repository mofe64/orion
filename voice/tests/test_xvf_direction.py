import math
import struct
import threading
import unittest

from orion_voice.direction import XvfDirectionEstimator
from orion_voice.xvf_control import PARAMETERS, XvfControl


class XvfDirectionTests(unittest.TestCase):
    def setUp(self):
        self.now = 0.0
        self.azimuth, self.energy = 0, 2
        self.calls = []
        self.control = XvfControl(self.transfer)
        self.estimator = XvfDirectionEstimator(0, 1, min_energy=1,
            clock=lambda: self.now, start_polling=False)

    def transfer(self, value, index, length):
        self.calls.append((value, index, length))
        name = 'AEC_AZIMUTH_VALUES' if value == 0x80 | PARAMETERS['AEC_AZIMUTH_VALUES'].command else 'AEC_SPENERGY_VALUES'
        value = math.radians(self.azimuth) if name == 'AEC_AZIMUTH_VALUES' else self.energy
        return bytes([0]) + struct.pack('<4f', 0, 0, 0, value)

    def sample(self, angle, count=5, energy=2):
        self.azimuth, self.energy = angle, energy
        for _ in range(count):
            self.estimator._poll_once(self.control)
            self.now += .05
        return self.estimator.observation()

    def test_offset_sign_and_wraparound(self):
        self.estimator.offset_deg = 350
        self.assertEqual(self.sample(80)['side'], 'left')
        self.estimator.reset()
        self.estimator.sign = -1
        self.assertEqual(self.sample(80)['side'], 'right')
        self.estimator.reset()
        self.assertEqual(self.sample(350)['side'], 'centre')
        for angle, side in [(179, 'left'), (181, 'right'), (-179, 'right'), (-181, 'left')]:
            self.estimator.offset_deg, self.estimator.sign = 0, 1
            self.estimator.reset()
            self.assertEqual(self.sample(angle)['side'], side)

    def test_front_side_and_rear_sectors(self):
        for angle, side in [(0, 'centre'), (29, 'centre'), (-29, 'centre'), (31, 'left'),
                            (-31, 'right'), (90, 'left'), (270, 'right'), (160, 'left'), (200, 'right')]:
            with self.subTest(angle=angle):
                self.estimator.reset()
                self.assertEqual(self.sample(angle)['side'], side)
        self.assertTrue(all(index == 33 and value & 0x80 for value, index, _ in self.calls))

    def test_low_energy_and_invalid_beam_ignored(self):
        for angle, energy in [(90, 0), (90, 1), (math.nan, 2), (math.inf, 2), (90, math.nan)]:
            self.estimator.reset()
            self.assertEqual(self.sample(angle, energy=energy)['side'], 'unknown')

    def test_vote_count_agreement_and_first_supporting_timestamp(self):
        self.assertEqual(self.sample(90, 4)['side'], 'unknown')
        self.assertEqual(self.sample(270, 2)['side'], 'unknown')
        self.assertEqual(self.sample(90, 2)['side'], 'left')
        self.assertEqual(self.estimator.observation()['confidence'], .75)
        self.assertEqual(self.estimator.observation()['observed_at'], 0)

    def test_expiry_reset_and_bounded_buffer(self):
        self.sample(90)
        self.now = 3.0
        self.estimator.accept(None)
        self.assertEqual(self.estimator.observation()['side'], 'unknown')
        self.sample(90, 100)
        self.assertLessEqual(len(self.estimator.samples), 64)
        self.estimator.reset()
        self.assertEqual(self.estimator.observation()['side'], 'unknown')

    def test_failure_clears_votes_logs_once_and_stops_reading(self):
        self.sample(90)
        def failure(*args):
            raise RuntimeError('USB failed')
        with self.assertLogs(level='WARNING') as logs:
            self.assertFalse(self.estimator._poll_once(XvfControl(failure)))
            self.assertFalse(self.estimator._poll_once(self.control))
        self.assertEqual(len(logs.output), 1)
        self.assertEqual(self.estimator.observation()['side'], 'unknown')

    def test_open_failure_and_missing_calibration_disable_direction(self):
        with self.assertLogs(level='WARNING') as logs:
            estimator = XvfDirectionEstimator()
        self.assertIsNone(estimator._thread)
        self.assertEqual(estimator.observation()['side'], 'unknown')
        def failure():
            raise RuntimeError('No board')
        with self.assertLogs(level='WARNING'):
            estimator = XvfDirectionEstimator(0, 1, open_control=failure)
            estimator._thread.join(1)
        self.assertEqual(estimator.observation()['side'], 'unknown')

    def test_audio_and_reset_do_not_wait_for_usb(self):
        entered, release = threading.Event(), threading.Event()
        def transfer(*args):
            entered.set()
            release.wait(2)
            return self.transfer(*args)
        estimator = XvfDirectionEstimator(0, 1, open_control=lambda: XvfControl(transfer))
        try:
            self.assertTrue(entered.wait(1))
            estimator.accept(None)
            estimator.reset()
            estimator.close()
            release.set()
            estimator._thread.join(1)
            self.assertFalse(estimator.samples)
        finally:
            release.set()
            estimator.close()
