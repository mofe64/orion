import unittest
from unittest.mock import Mock
import numpy as np
from orion_vision.behaviours import KEYS
from orion_vision.detect_faces import Face
from orion_vision.detect_objects import Object
from orion_vision.novelty import NoveltyTrack
import test_behaviours


def frame(dx=0):
    # Synthetic texture only; camera images never become fixtures.
    image = np.zeros((360, 640, 3), np.uint8)
    rng = np.random.default_rng(7)
    image[120:180, 180 + dx:240 + dx] = rng.integers(0, 256, (60, 60, 3), dtype=np.uint8)
    return image


def candidate():
    return dict(id='object:1', interest_key='object:book:0:-1', category='book',
                box=[178, 118, 64, 64], confidence=.9)


class NoveltyTests(unittest.TestCase):
    def test_flow_proves_continuity_and_rejects_gap_occlusion_and_feature_loss(self):
        track = NoveltyTrack(candidate(), frame(), 1)
        for step in range(1, 13):
            obj = track.advance(frame(step), 1 + step * 250_000_000, [])
            self.assertIsNotNone(obj)
        self.assertAlmostEqual(obj.box[0], 190, delta=1)
        self.assertIsNone(track.advance(frame(12), 4_500_000_001, []))
        track = NoveltyTrack(candidate(), frame(), 1)
        self.assertIsNone(track.advance(frame(), 250_000_001, [Face([180, 120, 60, 60], [], .9)]))
        track = NoveltyTrack(candidate(), frame(), 1)
        self.assertIsNone(track.advance(np.zeros((360, 640, 3), np.uint8), 250_000_001, []))
        with self.assertRaises(ValueError):
            NoveltyTrack(candidate(), np.zeros((360, 640, 3), np.uint8), 1)

    def pipeline(self):
        pipeline, client, detector, now = test_behaviours.BehaviourTests().pipeline()
        pipeline.configure_behaviours({**dict.fromkeys(KEYS, False), 'new_object_peek': True})
        detector.detect.return_value = []
        self.addCleanup(pipeline.close)
        return pipeline, client, detector, now

    def test_baseline_is_not_novel_dwell_has_fresh_geometry_and_voice_discards_it(self):
        pipeline, client, detector, now = self.pipeline()
        pipeline._background_objects(10_000_000_000, frame(), [], pipeline.generation)
        self.assertTrue(pipeline.object_baseline)
        self.assertIsNone(pipeline.novelty)
        now[0] = 70.
        detector.detect.return_value = [Object('book', candidate()['box'], .9)]
        pipeline._background_objects(70_000_000_000, frame(), [], pipeline.generation)
        self.assertIsNotNone(pipeline.novelty)
        for step in range(1, 13):
            stamp = 70_000_000_000 + step * 250_000_000
            pipeline._advance_novelty(stamp, frame(step), [], pipeline.generation)
            self.assertEqual(client.camera_pose.call_args.args, (stamp,))
        value = pipeline.novelty_observation
        self.assertEqual(value['capture_boottime_ns'] - value['since_boottime_ns'], 3_000_000_000)
        self.assertEqual(value['category'], 'book')
        self.assertNotIn('box', value)
        self.assertEqual(detector.detect.call_count, 2)  # No persistence detector reruns.
        client.find_allowed.return_value = False
        pipeline._advance_novelty(73_250_000_000, frame(12), [], pipeline.generation)
        self.assertIsNone(pipeline.novelty)
        self.assertIsNone(pipeline.novelty_observation)

    def test_existing_objects_and_reenable_do_not_create_novel_events(self):
        pipeline, _, detector, now = self.pipeline()
        detector.detect.return_value = [Object('book', candidate()['box'], .9)]
        for second in (10, 70):
            now[0] = second
            pipeline._background_objects(second * 1_000_000_000, frame(), [], pipeline.generation)
            self.assertIsNone(pipeline.novelty)
        old_key = pipeline.object_map.entries[0]['interest_key']
        pipeline.reset(False); pipeline.reset(True)
        now[0] = 130.
        pipeline._background_objects(130_000_000_000, frame(), [], pipeline.generation)
        self.assertIsNone(pipeline.novelty)
        self.assertEqual(pipeline.object_map.entries[0]['interest_key'], old_key)
