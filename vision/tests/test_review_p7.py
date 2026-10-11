import unittest
import numpy as np

from orion_vision.tracker import Tracker
from test_tracking import face


class SpeakerNoiseTests(unittest.TestCase):
    def test_one_pixel_noise_on_two_faces_only_real_mouth_motion_wins(self):
        for seed in (1, 5, 16):
            with self.subTest(seed=seed):
                tracker, rng = Tracker(), np.random.default_rng(seed)
                bearings = [{'azimuth': -.2, 'elevation': 0}, {'azimuth': .2, 'elevation': 0}]
                for index in range(32):
                    faces = [face(size=60), face(size=40, frontal=False)]
                    for person, current in enumerate(faces):
                        points = np.asarray(current.landmarks, dtype=float)
                        points[:2, 0] = [151, 169]  # 18 px eye separation at lores.
                        points += rng.uniform(-1, 1, points.shape) * (1 if person == 0 else .65)
                        if person == 1 and index >= 16:
                            points[3:, 1] += 2 if index % 2 else -2
                        current.landmarks = points.tolist()
                    tracker.update(1_000_000_000 + index * 100_000_000, faces, bearings)
                result = tracker.speaker(2_600_000_000, 4_100_000_000)
                self.assertEqual(result['strategy'], 'mouth_motion')
                self.assertEqual(result['speaker']['id'], 2)

    def test_noise_only_and_two_equal_movers_fall_back_to_frontality(self):
        bearings = [{'azimuth': -.2, 'elevation': 0}, {'azimuth': .2, 'elevation': 0}]
        for speaking in (False, True):
            with self.subTest(speaking=speaking):
                tracker, rng = Tracker(), np.random.default_rng(1)
                for index in range(32):
                    faces = [face(size=60), face(size=40, frontal=False)]
                    # Equal evidence has no speaker winner, even above threshold.
                    noise = rng.uniform(-1, 1, (5, 2))
                    for current in faces:
                        points = np.asarray(current.landmarks, dtype=float) + noise
                        if speaking and index >= 16:
                            points[3:, 1] += 3 if index % 2 else -3
                        current.landmarks = points.tolist()
                    tracker.update(1_000_000_000 + index * 100_000_000, faces, bearings)
                result = tracker.speaker(2_600_000_000, 4_100_000_000)
                self.assertEqual((result['strategy'], result['speaker']['id']), ('looking_at_orion', 1))
