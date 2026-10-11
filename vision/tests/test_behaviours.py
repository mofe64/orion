import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock

import numpy as np

from orion_vision.behaviours import KEYS, ObjectMap, settings
from orion_vision.detect_faces import Face
from orion_vision.detect_objects import Object
from orion_vision.pipeline import FacePipeline
from test_objects import CAMERA, intrinsics


class BehaviourTests(unittest.TestCase):
    def test_snapshot_racing_clear_cannot_restore_entries(self):
        mapping = ObjectMap()
        class ClearDuringRead:
            def __int__(self):
                mapping.clear()
                return 1
        mapping.entries = [{'capture_boottime_ns': ClearDuringRead(), 'id': 'old'}]
        mapping.snapshot(2)
        self.assertEqual(mapping.entries, [])

    def test_preferences_are_opt_in_and_any_corruption_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'behaviours.json'
            off = dict.fromkeys(KEYS, False)
            self.assertEqual(settings(path), off)
            path.write_text('{"object_glances":true}')
            self.assertEqual(settings(path), {**off, 'object_glances': True})
            for content in ('broken', '[]', 'null', '{"object_glances":true,"mutual_gaze":1}'):
                path.write_text(content)
                self.assertEqual(settings(path), off)

    def test_map_matches_base_bearings_expires_and_keeps_distinct_objects(self):
        mapping = ObjectMap()
        def obj(az, stamp):
            return dict(category='book', azimuth=az, elevation=-.3,
                        capture_boottime_ns=str(stamp), point=[0, -.3, 0])
        mapping.update([obj(.2, 1), obj(.5, 1)], 1)
        ids = [entry['id'] for entry in mapping.snapshot(1)]
        mapping.update([obj(.21, 61_000_000_000)], 61_000_000_000)
        self.assertEqual(mapping.entries[0]['id'], ids[0])
        self.assertEqual(len(mapping.snapshot(600_000_000_001)), 1)
        self.assertEqual(mapping.snapshot(661_000_000_000), [])
        mapping.clear(); mapping.update([obj(.2, 700_000_000_000)], 700_000_000_000)
        self.assertNotIn(mapping.entries[0]['id'], ids)

    def pipeline(self):
        now = [10.]
        client = Mock()
        client.background_ready.return_value = client.find_allowed.return_value = True
        client.camera_pose.return_value = CAMERA
        objects = Mock()
        objects.detect.return_value = [Object('person', [0, 0, 20, 20], .9),
            Object('book', [100, 100, 60, 60], .9), Object('cup', [300, 200, 30, 30], .8)]
        pipeline = FacePipeline(Mock(), intrinsics(), client, clock=lambda: now[0],
                                boot_clock=lambda: int(now[0] * 1e9), objects=objects)
        pipeline.reset(True)
        pipeline.configure_behaviours({**dict.fromkeys(KEYS, False), 'object_glances': True})
        return pipeline, client, objects, now

    def test_background_serial_pass_people_exclusion_cooldown_and_capture_pose(self):
        pipeline, client, detector, now = self.pipeline()
        faces = [Face([105, 105, 15, 15], [], .9)]
        calls = []
        client.camera_pose.side_effect = lambda stamp: calls.append(('pose', stamp)) or CAMERA
        detector.detect.side_effect = lambda buffer: calls.append(('inference', None)) or [
            Object('person', [0, 0, 20, 20], .9), Object('book', [100, 100, 60, 60], .9),
            Object('cup', [300, 200, 30, 30], .8)]
        pipeline._background_objects(10_000_000_000, np.zeros((360, 640, 3), np.uint8), faces, pipeline.generation)
        self.assertEqual(calls[0], ('pose', 10_000_000_000))
        self.assertEqual([entry['category'] for entry in pipeline.object_map.entries], ['cup'])
        for _ in range(10):
            pipeline._background_objects(10_000_000_000, None, [], pipeline.generation)
        self.assertEqual(detector.detect.call_count, 1)
        now[0] = 70.
        client.find_allowed.return_value = False
        pipeline._background_objects(70_000_000_000, None, [], pipeline.generation)
        self.assertEqual(detector.detect.call_count, 1)
        pipeline.reset(False)
        self.assertEqual(pipeline.object_map.entries, [])

    def test_privacy_change_during_inference_discards_map_and_cooldown_survives(self):
        pipeline, client, detector, now = self.pipeline()
        def detect(_):
            pipeline.reset(False)
            return [Object('cup', [300, 200, 30, 30], .8)]
        detector.detect.side_effect = detect
        pipeline._background_objects(10_000_000_000, None, [], pipeline.generation)
        self.assertEqual(pipeline.object_map.entries, [])
        pipeline.reset(True)
        pipeline._background_objects(10_000_000_000, None, [], pipeline.generation)
        self.assertEqual(detector.detect.call_count, 1)

    def test_background_pass_holds_only_inference_worker_and_privacy_can_clear_it(self):
        from threading import Event
        pipeline, client, detector, now = self.pipeline()
        entered, release = Event(), Event()
        pipeline.detector.detect.return_value = []
        client.voice_active.return_value = False
        def detect(_):
            entered.set()
            release.wait(2)
            return [Object('cup', [300, 200, 30, 30], .8)]
        detector.detect.side_effect = detect
        self.addCleanup(pipeline.close)
        self.addCleanup(release.set)
        pipeline.start()
        frame = np.zeros((360, 640, 3), np.uint8)
        pipeline.submit(10_000_000_000, frame)
        self.assertTrue(entered.wait(1))
        for step in range(10): pipeline.submit(10_000_000_001 + step, frame)
        self.assertGreater(pipeline.dropped, 0)
        self.assertEqual(pipeline.detector.detect.call_count, 1)
        pipeline.reset(False)
        self.assertEqual(pipeline.object_map.entries, [])
        release.set()

    def test_metadata_carries_only_fresh_coarse_faces_and_no_pixels(self):
        pipeline, client, _, _ = self.pipeline()
        track = Mock(id=7, state=[.2, -.1])
        track.face.looking_at_orion = True
        pipeline._surroundings(10_000_000_000, [track])
        value = client.surroundings.call_args.args[0]
        self.assertEqual(value['faces'], [{'track_id': 7, 'azimuth': .2, 'elevation': -.1, 'frontal': True}])
        self.assertFalse(value['looking_away'])
        self.assertEqual(value['objects'], [])
        self.assertNotIn('image', json.dumps(value))
