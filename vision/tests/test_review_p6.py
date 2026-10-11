import copy
import math
from pathlib import Path
import tempfile
from threading import Thread
import time
import unittest
from unittest.mock import MagicMock, patch

import numpy as np

from orion_vision.intrinsics import SIZE, Intrinsics, lores_bgr
from orion_vision.gaze_client import GazeClient
from orion_vision.pipeline import FacePipeline
from orion_vision.sources import Picamera2Source
from orion_vision.tracker import Tracker, angle_delta
from test_tracking import FakeRuntime, eventually, face, intrinsics


GEOMETRY = {'scaler_crop': [0, 0, 4608, 2592],
    'sensor_mode': {'output_size': [2304, 1296], 'bit_depth': 10},
    'flips': {'hflip': False, 'vflip': False}, 'lores_stride': SIZE[0] + 64}


class GeometryTests(unittest.TestCase):
    def test_lores_rejects_wrong_resolution_dtype_and_short_stride_before_crop(self):
        rows, stride = SIZE[1] * 3 // 2, GEOMETRY['lores_stride']
        for buffer in (np.zeros((rows * 2, SIZE[0]), np.uint8), np.zeros((rows, SIZE[0] - 2), np.uint8),
                       np.zeros((SIZE[1], SIZE[0] + 1, 3), np.uint8), np.zeros((rows, stride), np.float32)):
            with self.subTest(shape=buffer.shape), self.assertRaises(ValueError):
                lores_bgr(buffer)
        self.assertEqual(lores_bgr(np.zeros((rows, stride), np.uint8)).shape, (SIZE[1], SIZE[0], 3))

    def test_pipeline_drops_changed_crop_sensor_flip_and_stride(self):
        model = Intrinsics.camera_module_3_wide()
        model.geometry['lores_stride'] = GEOMETRY['lores_stride']
        pipeline = FacePipeline(MagicMock(), model, MagicMock())
        pipeline.reset(True)
        buffer = np.zeros((SIZE[1] * 3 // 2, GEOMETRY['lores_stride']), np.uint8)
        geometry = {key: value for key, value in GEOMETRY.items() if key != 'scaler_crop'}
        pipeline.submit(100, buffer, {'ScalerCrop': GEOMETRY['scaler_crop']}, geometry)
        self.assertIsNotNone(pipeline.pending)
        pipeline.pending = None
        for key, value in [('scaler_crop', [1, 0, 4608, 2592]),
                           ('sensor_mode', {'output_size': [1536, 864], 'bit_depth': 10}),
                           ('flips', {'hflip': True, 'vflip': False}), ('lores_stride', SIZE[0])]:
            changed = copy.deepcopy(GEOMETRY)
            changed[key] = value
            pipeline.submit(200, buffer, {'ScalerCrop': changed.pop('scaler_crop')}, changed)
            self.assertIsNone(pipeline.pending, key)
        pipeline.submit(300, np.zeros((SIZE[1] * 3 // 2, SIZE[0]), np.uint8),
            {'ScalerCrop': GEOMETRY['scaler_crop']}, geometry)
        self.assertIsNone(pipeline.pending)  # Buffer stride must match metadata too.
        self.assertEqual(pipeline.dropped, 5)

    def test_source_rejects_flips_and_negotiated_sensor_mismatch(self):
        for hflip, vflip in [(True, False), (False, True), (True, True)]:
            with self.assertRaisesRegex(ValueError, 'flip'):
                Picamera2Source(hflip=hflip, vflip=vflip)
        source = Picamera2Source()
        source.expected_geometry = copy.deepcopy(GEOMETRY)
        camera, module = MagicMock(), MagicMock()
        module.Picamera2.return_value = camera
        camera.camera_configuration.return_value = {
            'sensor': {'output_size': (1536, 864), 'bit_depth': 10},
            'lores': {'size': SIZE, 'format': 'YUV420', 'stride': GEOMETRY['lores_stride']}}
        with patch.dict('sys.modules', picamera2=module, libcamera=MagicMock()):
            with self.assertRaisesRegex(ValueError, 'geometry'):
                source.start(MagicMock(), lambda: False, lambda: True)
        camera.start.assert_not_called()
        camera.close.assert_called_once()


class PipelineReviewTests(unittest.TestCase):
    def test_default_gaze_rejection_preserves_ids_and_backs_off_then_recovers(self):
        with tempfile.TemporaryDirectory() as directory:
            server = FakeRuntime(Path(directory) / 'runtime.sock', reject_gaze=True)
            thread = Thread(target=server.serve_forever, kwargs={'poll_interval': .01}, daemon=True)
            thread.start()
            now, clock = [1_000_000_000], [0.]
            pipeline = FacePipeline(MagicMock(detect=lambda _: [face()]), intrinsics(),
                GazeClient(server.server_address), clock=lambda: clock[0], boot_clock=lambda: now[0])
            pipeline.reset(True); pipeline.start()
            try:
                for index in range(5):
                    now[0] += 100_000_000; clock[0] += .11
                    server.head_at_capture[now[0]] = 0
                    pipeline.submit(now[0], 1)
                    eventually(lambda: pipeline.inferences == index + 1)
                    eventually(lambda: len(pipeline.observations()) == 1)
                    self.assertEqual(pipeline.observations()[0]['id'], 1)
                self.assertEqual(sum(c.startswith('gaze target') for c in server.commands), 1)
                self.assertEqual(sum(c == 'gaze clear' for c in server.commands), 0)
                server.reject_gaze = False
                clock[0] += 3.1; now[0] += 100_000_000
                server.head_at_capture[now[0]] = 0
                pipeline.submit(now[0], 1)
                eventually(lambda: sum(c.startswith('gaze target') for c in server.commands) == 2)
                self.assertEqual(pipeline.observations()[0]['id'], 1)
            finally:
                pipeline.close(); server.shutdown(); server.server_close(); thread.join(1)

    def test_one_missed_detection_keeps_live_target_and_empty_room_clears_once(self):
        now, clock = [1_000_000_000], [0.]
        detector, client = MagicMock(), MagicMock()
        detector.detect.return_value = [face()]
        client.voice_active.return_value = False
        client.bearings.return_value = [{'azimuth': .2, 'elevation': 0}]
        pipeline = FacePipeline(detector, intrinsics(), client,
            clock=lambda: clock[0], boot_clock=lambda: now[0])
        pipeline.reset(True); pipeline.start()
        self.addCleanup(pipeline.close)
        def infer(delta):
            before = pipeline.inferences
            now[0] += delta; clock[0] += .11
            pipeline.submit(now[0], 1)
            eventually(lambda: pipeline.inferences == before + 1)
            time.sleep(.01)
        infer(1)
        first_id = pipeline.observations()[0]['id']
        detector.detect.return_value = []
        infer(100_000_000)
        self.assertEqual(pipeline.observations()[0]['id'], first_id)
        self.assertFalse(client.clear.called)
        self.assertEqual(client.target.call_count, 1)  # Do not replay a capture timestamp.
        infer(1_000_000_001)
        self.assertEqual(client.clear.call_count, 1)
        for _ in range(3):
            infer(100_000_000)
        self.assertEqual(client.clear.call_count, 1)


class CoordinateTests(unittest.TestCase):
    def test_real_ray_sign_wraparound_and_constant_velocity_prediction(self):
        with tempfile.TemporaryDirectory() as directory:
            server = FakeRuntime(Path(directory) / 'runtime.sock')
            thread = Thread(target=server.serve_forever, kwargs={'poll_interval': .01}, daemon=True)
            thread.start()
            try:
                client, tracker = GazeClient(server.server_address), Tracker()
                identity = None
                for index in range(20):
                    stamp = 1_000_000_000 + index * 100_000_000
                    world = 3.0 + index * .025
                    yaw = 3.1
                    server.head_at_capture[stamp] = yaw
                    ray = np.array([[math.tan(yaw - world), 0., 1.]])
                    ray /= np.linalg.norm(ray, axis=1)[:, None]
                    bearing = client.bearings(stamp, ray)
                    self.assertAlmostEqual(angle_delta(bearing[0]['azimuth'] - world), 0., places=6)
                    track, = tracker.update(stamp, [face()], bearing)
                    identity = identity or track.id
                    self.assertEqual(track.id, identity)
                self.assertAlmostEqual(track.state[2], .25, delta=.01)
                predicted = track.state[0] + .5 * track.state[2]
                self.assertAlmostEqual(angle_delta(predicted - (world + .125)), 0., delta=.01)
                self.assertLess(tracker.observations(stamp)[0]['azimuth'], 0.)
            finally:
                server.shutdown(); server.server_close(); thread.join(1)
