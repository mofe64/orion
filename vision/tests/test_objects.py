import tempfile
from pathlib import Path
from threading import Event, Thread
import time
import unittest
from unittest.mock import MagicMock, patch

import numpy as np

from orion_vision.detect_objects import CATEGORIES, Object, _NativeYoloNCNN, decode
from orion_vision.find import FindError, locate
from orion_vision.gaze_client import GazeClient
from orion_vision.intrinsics import SIZE, Intrinsics
from orion_vision.pipeline import FacePipeline
from orion_vision.service import Vision
from test_capture_control import QuietSource
from test_tracking import eventually


CAMERA = {'position': [0, 0, .3], 'rotation': [[1, 0, 0], [0, 0, -1], [0, 1, 0]]}


def intrinsics():
    return Intrinsics([[300, 0, 320], [0, 300, 180], [0, 0, 1]], [0, 0, 0, 0])


class GILHoldingDetector:
    def __init__(self, entered):
        self.entered = entered

    def detect(self, pixels):
        import sys
        sys.setswitchinterval(30.)
        self.entered.set()
        until = time.monotonic() + 2.5
        while time.monotonic() < until:
            pass
        return []


class ObjectTests(unittest.TestCase):
    def test_child_timeout_releases_cpu_before_return_and_reports_duration(self):
        import multiprocessing
        from orion_vision.inference_process import InferenceProcess
        entered = multiprocessing.get_context('spawn').Event()
        detector = InferenceProcess(GILHoldingDetector, entered, timeout=.1)
        self.addCleanup(detector.close)
        with self.assertLogs('orion_vision.inference_process', level='INFO') as logged:
            with self.assertRaises(TimeoutError):
                detector.detect(np.zeros((SIZE[1], SIZE[0], 3), np.uint8))
        self.assertTrue(entered.is_set())
        self.assertFalse(detector.process.is_alive())
        self.assertFalse(detector.available)
        self.assertIn('duration_ms=', logged.output[0])
        with self.assertRaisesRegex(RuntimeError, 'unavailable'):
            detector.detect(None)

    def test_letterbox_decoding_classwise_nms_and_bounded_results(self):
        rows = np.zeros((84, 2100), dtype=np.float32)
        book, cup = CATEGORIES.index('book'), CATEGORIES.index('cup')
        for i, (x, category, score) in enumerate([(160, book, .9), (162, book, .8),
                                                 (160, cup, .7), (80, book, .6)]):
            rows[:4, i] = [x, 160, 40, 20]
            rows[4 + category, i] = score
        result = decode(rows)
        self.assertEqual([obj.category for obj in result], ['book', 'cup', 'book'])
        self.assertEqual(result[0].box, [280., 160., 80., 40.])
        rows[0, 0] = np.nan
        self.assertEqual(len(decode(rows)), 3)
        with self.assertRaisesRegex(ValueError, 'shape'):
            decode(rows.T)

    def test_replay_image_preserves_aspect_and_rgb_order(self):
        detector = _NativeYoloNCNN.__new__(_NativeYoloNCNN)
        detector.ncnn, detector.net = MagicMock(), MagicMock()
        detector.net.create_extractor.return_value.__enter__.return_value.input.return_value = 0
        detector.net.create_extractor.return_value.__enter__.return_value.extract.return_value = (
            0, np.zeros((84, 2100), dtype=np.float32))
        pixels = np.full((SIZE[1], SIZE[0], 3), [250, 20, 10], dtype=np.uint8)
        self.assertEqual(detector.detect(pixels), [])
        args = detector.ncnn.Mat.from_pixels.call_args.args
        square = args[0]
        self.assertEqual(square.shape, (320, 320, 3))
        self.assertEqual(square[69, 0].tolist(), [114] * 3)
        self.assertEqual(square[70, 0].tolist(), [10, 20, 250])
        self.assertEqual(square[249, 0].tolist(), [10, 20, 250])
        self.assertEqual(square[250, 0].tolist(), [114] * 3)

    def test_capture_transform_uses_translation_and_rejects_upward_ray(self):
        model = intrinsics()
        # A ray 45 degrees down meets the table 30 cm forward from this camera.
        import math
        pixel = [320, 180 - 300 * math.pi / 4]
        obj = Object('book', [pixel[0]-10, pixel[1]-10, 20, 20], .9)
        point = locate([obj], model, CAMERA, 123)[0]['point']
        np.testing.assert_allclose(point, [0, -.3, 0], atol=.002)
        obj.box[1] = 220
        self.assertIsNone(locate([obj], model, CAMERA, 123)[0]['point'])
        with self.assertRaisesRegex(ValueError, 'transform'):
            locate([obj], model, {**CAMERA, 'rotation': [[0]*3]*3}, 123)

    def test_voice_permission_requires_owned_interval_and_background_stays_blocked(self):
        client = GazeClient()
        with patch.object(client, 'command') as command:
            command.return_value = {'voice': {'session': 'owner'}, 'vision_session': 'owner'}
            self.assertTrue(client.find_allowed('owner'))
            self.assertFalse(client.find_allowed('retired'))
            self.assertFalse(client.find_allowed())
            command.return_value = {'voice': {'session': None}}
            self.assertTrue(client.find_allowed())
            self.assertFalse(client.find_allowed('owner'))

    def pipeline(self, object_detect):
        faces, objects, client = MagicMock(), MagicMock(), MagicMock()
        faces.detect.return_value = []
        objects.detect.side_effect = object_detect
        client.voice_active.return_value = False
        client.find_allowed.return_value = True
        client.camera_pose.return_value = CAMERA
        pipeline = FacePipeline(faces, intrinsics(), client, objects=objects)
        pipeline.start()
        pipeline.reset(True)
        self.addCleanup(pipeline.close)
        return pipeline, faces, client

    def start_find(self, pipeline, **kwargs):
        result = []
        def run():
            try:
                result.append(pipeline.find('book', **kwargs))
            except Exception as error:
                result.append(error)
        thread = Thread(target=run)
        thread.start()
        self.addCleanup(lambda: thread.join(2))
        eventually(lambda: pipeline.find_job is not None)
        return result, thread

    def test_one_worker_pauses_faces_keeps_latest_frame_and_snapshots_pose_first(self):
        entered, release = Event(), Event()
        def detect(buffer):
            client.camera_pose.assert_called_once()
            entered.set()
            release.wait(2)
            return [Object('book', [300, 50, 40, 40], .9)]
        pipeline, faces, client = self.pipeline(detect)
        self.addCleanup(release.set)
        result, thread = self.start_find(pipeline)
        pixels = np.zeros((SIZE[1], SIZE[0], 3), np.uint8)
        pipeline.submit(pipeline.boot_clock(), pixels)
        self.assertTrue(entered.wait(1))
        for _ in range(20):
            pipeline.submit(pipeline.boot_clock(), pixels)
        self.assertEqual(faces.detect.call_count, 0)
        self.assertGreaterEqual(pipeline.dropped, 19)
        with self.assertRaisesRegex(FindError, 'already running'):
            pipeline.find('book')
        release.set(); thread.join(2)
        self.assertEqual(result[0]['status'], 'found')
        eventually(lambda: faces.detect.called)

    def test_privacy_revokes_slow_result_and_voice_denial_runs_no_model(self):
        entered, release = Event(), Event()
        def detect(buffer):
            entered.set(); release.wait(2)
            return [Object('book', [300, 50, 40, 40], .9)]
        pipeline, _, client = self.pipeline(detect)
        self.addCleanup(release.set)
        result, thread = self.start_find(pipeline)
        pipeline.submit(pipeline.boot_clock(), np.zeros((SIZE[1], SIZE[0], 3), np.uint8))
        self.assertTrue(entered.wait(1))
        pipeline.reset(False)
        self.assertTrue(thread.is_alive())  # CPU exclusion remains until native return.
        release.set()
        thread.join(1)
        self.assertIsInstance(result[0], FindError)
        self.assertEqual(result[0].code, 'camera_changed')
        eventually(lambda: pipeline.find_job is None)
        pipeline.reset(True)
        client.find_allowed.return_value = False
        result, thread = self.start_find(pipeline)
        pipeline.submit(pipeline.boot_clock(), np.zeros((SIZE[1], SIZE[0], 3), np.uint8))
        thread.join(1)
        self.assertEqual(result[0].code, 'voice_active')
        self.assertEqual(pipeline.objects.detect.call_count, 1)

    def test_behaviour_changes_do_not_invalidate_explicit_find(self):
        entered, release = Event(), Event()
        def detect(buffer):
            entered.set(); release.wait(2)
            return [Object('book', [300, 50, 40, 40], .9)]
        pipeline, _, _ = self.pipeline(detect)
        self.addCleanup(release.set)
        result, thread = self.start_find(pipeline)
        pipeline.submit(pipeline.boot_clock(), np.zeros((SIZE[1], SIZE[0], 3), np.uint8))
        self.assertTrue(entered.wait(1))
        generation = pipeline.capture_generation
        pipeline.configure_behaviours({**pipeline.behaviours, 'mutual_gaze': True})
        self.assertEqual(pipeline.capture_generation, generation)
        self.assertNotEqual(pipeline.generation, generation)
        release.set(); thread.join(1)
        self.assertEqual(result[0]['status'], 'found')

    def test_expired_lookup_holds_cpu_ownership_until_native_inference_finishes(self):
        entered, release = Event(), Event()
        def detect(buffer):
            entered.set(); release.wait(2)
            return []
        pipeline, _, _ = self.pipeline(detect)
        self.addCleanup(release.set)
        result, thread = self.start_find(pipeline, timeout=.08)
        pipeline.submit(pipeline.boot_clock(), np.zeros((SIZE[1], SIZE[0], 3), np.uint8))
        self.assertTrue(entered.wait(1))
        eventually(lambda: pipeline.find_job.error is not None)
        self.assertTrue(thread.is_alive())
        self.assertEqual(result, [])
        with self.assertRaisesRegex(FindError, 'already running'):
            pipeline.find('book')
        release.set(); thread.join(1)
        self.assertEqual(result[0].code, 'find_timeout')

    def test_capture_watchdog_is_independent_of_blocked_object_inference(self):
        import multiprocessing
        from orion_vision.inference_process import InferenceProcess
        entered = multiprocessing.get_context('spawn').Event()
        detector = InferenceProcess(GILHoldingDetector, entered)
        self.addCleanup(detector.close)
        pipeline, _, _ = self.pipeline(detector.detect)
        # Vision starts its own worker; reuse this one through the privacy loop.
        pipeline.start = lambda: None
        with tempfile.TemporaryDirectory() as directory:
            clock = [0.]
            source = QuietSource()
            vision = Vision(source, Path(directory) / 'camera.json', pipeline=pipeline, clock=lambda: clock[0])
            vision.start()
            try:
                eventually(lambda: source.starts == 1)
                result, thread = self.start_find(pipeline)
                source.emit(pipeline.boot_clock())
                self.assertTrue(entered.wait(1))
                for _ in range(8):
                    clock[0] += .5
                    source.emit(pipeline.boot_clock())
                    time.sleep(.015)
                self.assertEqual(vision.camera_restarts, 0)
                self.assertEqual(source.starts, 1)
                self.assertTrue(thread.is_alive(), 'Capture advanced during actual GIL-holding inference')
                thread.join(4)
                self.assertEqual(result[0]['status'], 'not_found')
            finally:
                vision.close()
