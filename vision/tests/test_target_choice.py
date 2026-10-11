import unittest
from unittest.mock import MagicMock

from orion_vision.pipeline import FacePipeline
from orion_vision.tracker import MAX_AGE_NS
from test_tracking import face, intrinsics


class SteadyTargetTests(unittest.TestCase):
    def setUp(self):
        self.pipeline = FacePipeline(MagicMock(), intrinsics(), MagicMock())
        self.stamp = 1_000_000_000
        self.bearings = [{'azimuth': -.4, 'elevation': 0}, {'azimuth': .4, 'elevation': 0}]

    def update(self, sizes, frontal=True):
        self.stamp += 100_000_000
        faces = [face(size=sizes[0]), face(size=sizes[1], frontal=frontal)]
        self.pipeline.tracker.update(self.stamp, faces, self.bearings)
        return self.pipeline._choose_target(self.pipeline.tracker.tracks, self.stamp)

    def test_similar_faces_alternating_area_keep_the_same_target(self):
        first = self.update((51, 50)).id
        for index in range(30):
            self.assertEqual(self.update((50, 51) if index % 2 else (51, 50)).id, first)

    def test_larger_looking_challenger_must_hold_advantage_for_one_second(self):
        first = self.update((50, 49)).id
        for _ in range(10):
            self.assertEqual(self.update((50, 70)).id, first)
        chosen = self.update((50, 70))
        self.assertNotEqual(chosen.id, first)
        self.assertEqual(chosen.face.box[2], 70)

    def test_area_advantage_below_one_point_five_never_switches(self):
        first = self.update((50, 49)).id
        # 61² / 50² = 1.4884, just below the area threshold.
        for _ in range(20):
            self.assertEqual(self.update((50, 61)).id, first)
        for _ in range(10):
            self.assertEqual(self.update((50, 62)).id, first)
        self.assertNotEqual(self.update((50, 62)).id, first)

    def test_lost_advantage_resets_challenge_and_nonlooking_face_cannot_switch(self):
        first = self.update((50, 49)).id
        for _ in range(6):
            self.assertEqual(self.update((50, 70)).id, first)
        self.assertEqual(self.update((50, 70), frontal=False).id, first)
        for _ in range(10):
            self.assertEqual(self.update((50, 70)).id, first)
        self.assertNotEqual(self.update((50, 70)).id, first)

    def test_lost_target_switches_without_waiting_and_reset_forgets_choice(self):
        first = self.update((51, 50)).id
        other = self.pipeline.tracker.tracks[1].id
        for _ in range(11):
            self.stamp += 100_000_000
            self.pipeline.tracker.update(self.stamp, [face(size=50)], self.bearings[1:])
            fresh = [track for track in self.pipeline.tracker.tracks
                if self.stamp - track.capture_ns <= MAX_AGE_NS]
            chosen = self.pipeline._choose_target(fresh, self.stamp)
        self.assertEqual(chosen.id, other)
        self.assertNotEqual(chosen.id, first)
        self.pipeline.reset(False)
        self.assertIsNone(self.pipeline.target_track_id)
