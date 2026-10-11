import json
import math
from pathlib import Path
import socketserver
import tempfile
from threading import Event, Thread
import time
import unittest
from unittest.mock import MagicMock, patch

import cv2
import numpy as np

from orion_vision.calibration import Intrinsics, checkerboard_points, fit, lores_bgr
from orion_vision.detect_faces import Face, MODEL_SHA256, YuNet
from orion_vision.gaze_client import GazeClient, boottime_ns
from orion_vision.pipeline import FacePipeline
from orion_vision.sources import Picamera2Source
from orion_vision.tracker import Tracker, choose_track


def intrinsics():
    return Intrinsics([[125, 0, 160], [0, 125, 120], [0, 0, 1]], [-.05, .005, 0, 0], .1)


def face(centre=(160, 120), size=40, frontal=True):
    x, y = centre
    return Face([x - size / 2, y - size / 2, size, size],
        [[x-10, y-8], [x+10, y-8], [x if frontal else x+9, y], [x-7, y+8], [x+7, y+8]], .95)


def eventually(predicate, seconds=2):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(.005)
    raise AssertionError("Condition did not occur")


class CalibrationTests(unittest.TestCase):
    def test_fisheye_projection_round_trip_including_edges(self):
        calibration = intrinsics()
        points = np.array([[math.tan(a), math.tan(e), 1]
            for a in np.linspace(-.95, .95, 17) for e in np.linspace(-.7, .7, 9)], dtype=float)
        pixels, _ = cv2.fisheye.projectPoints(points.reshape(-1, 1, 3), np.zeros(3), np.zeros(3),
            calibration.matrix, calibration.distortion)
        rays = calibration.rays(pixels)
        expected = points / np.linalg.norm(points, axis=1)[:, None]
        errors = np.arccos(np.clip(np.sum(rays * expected, axis=1), -1, 1))
        self.assertLess(float(max(errors)), math.radians(.2))

    def test_fits_synthetic_distinct_checkerboards_and_saves_only_numbers(self):
        calibration = intrinsics()
        board = checkerboard_points(9, 6, .025)
        corners = []
        for i in range(18):
            pixels, _ = cv2.fisheye.projectPoints(board, np.array([.1 * (i % 3), -.15 + .07*(i % 5), .03*i]),
                np.array([-.15 + .03*(i % 6), -.1 + .02*(i % 4), .4 + .03*i]),
                calibration.matrix, calibration.distortion)
            corners.append(pixels)
        fitted = fit(corners, 9, 6, .025)
        fitted.geometry = {'scaler_crop': [0, 0, 4608, 2592],
            'sensor_mode': {'output_size': [1536, 864], 'bit_depth': 10},
            'flips': {'hflip': False, 'vflip': False}, 'lores_stride': 384}
        self.assertLess(fitted.rms, .01)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "camera-v2.json"
            fitted.save(path)
            loaded = Intrinsics.load(path)
            np.testing.assert_allclose(loaded.matrix, fitted.matrix)
            self.assertEqual(list(Path(directory).iterdir()), [path])
            with self.assertRaises(ValueError):
                Intrinsics(fitted.matrix, fitted.distortion, .50001)
            with patch('orion_vision.calibration.cv2.fisheye.calibrate', return_value=(.51, fitted.matrix, fitted.distortion, [], [])):
                with self.assertRaises(ValueError):
                    fit(corners, 9, 6, .025)
            self.assertEqual(Intrinsics.load(path).rms, loaded.rms)

    def test_yuv_stride_crop_after_conversion_and_only_callback_copy(self):
        buffer = np.zeros((360, 384), dtype=np.uint8)
        self.assertEqual(lores_bgr(buffer).shape, (240, 320, 3))
        camera = MagicMock()
        module = MagicMock()
        module.Picamera2.return_value = camera
        camera.camera_configuration.return_value = {
            'sensor': {'output_size': (1536, 864), 'bit_depth': 10},
            'lores': {'size': (320, 240), 'format': 'YUV420', 'stride': 384}}
        with patch.dict('sys.modules', picamera2=module, libcamera=MagicMock()):
            source = Picamera2Source()
            self.addCleanup(source.close)
            publish = MagicMock()
            source.start(publish, lambda: False, lambda: True)
            self.assertEqual(camera.create_video_configuration.call_args.kwargs['lores'],
                {'size': (320, 240), 'format': 'YUV420'})
            request = MagicMock()
            request.make_array.return_value = buffer
            camera.post_callback(request)
            request.make_array.assert_called_once_with('lores')
            self.assertIsNone(publish.call_args.args[0].rgb)
            self.assertIs(publish.call_args.args[0].lores, buffer)

    def test_model_requires_pinned_sha_and_frontality_is_symmetry(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'wrong.onnx'
            path.write_bytes(b'wrong asset')
            with self.assertRaisesRegex(ValueError, 'SHA-256'):
                YuNet(path)
        self.assertEqual(len(MODEL_SHA256), 64)
        self.assertTrue(face().looking_at_orion)
        self.assertFalse(face(frontal=False).looking_at_orion)


class FakeRuntime(socketserver.UnixStreamServer):
    def __init__(self, path, reject_gaze=False):
        self.reject_gaze = reject_gaze
        self.head_at_capture = {}
        self.commands = []
        class Handler(socketserver.StreamRequestHandler):
            def handle(handler):
                command = handler.rfile.readline().decode().strip()
                self.commands.append(command)
                if command.startswith('camera-rays '):
                    value = json.loads(command.removeprefix('camera-rays '))
                    yaw = self.head_at_capture[value['capture_boottime_ns']]
                    # Optical +X maps to base +X at yaw zero; Bearing uses
                    # atan2(-base_x, -base_y), so image-right is negative azimuth.
                    bearings = [{'azimuth': (yaw - math.atan2(ray[0], ray[2]) + math.pi) % (2*math.pi) - math.pi,
                        'elevation': math.atan2(-ray[1], math.hypot(ray[0], ray[2]))} for ray in value['rays']]
                    result = {'ok': True, 'bearings': bearings, 'pose_age_ms': 0}
                elif command.startswith('gaze target'):
                    result = {'ok': not self.reject_gaze, 'error': 'Gaze requires --gaze on'}
                elif command == 'gaze status':
                    result = {'ok': True, 'gaze': {'enabled': not self.reject_gaze}}
                elif command == 'voice status':
                    result = {'ok': True, 'voice': {'session': None}}
                else:
                    result = {'ok': True}
                handler.wfile.write(json.dumps(result).encode() + b'\n')
        super().__init__(str(path), Handler)


class TrackerTests(unittest.TestCase):
    def test_speaker_uses_relative_mouth_motion_then_frontality_then_nearest(self):
        tracker = Tracker()
        bearings = [{'azimuth': -.2, 'elevation': 0}, {'azimuth': .2, 'elevation': 0}]
        tracks = tracker.update(1_000_000_000, [face(size=30), face(size=80, frontal=False)], bearings)
        first_id, second_id = [track.id for track in tracks]
        moved = face(size=80, frontal=False)
        moved.landmarks[3][1] += 4
        moved.landmarks[4][1] += 4
        tracker.update(1_100_000_000, [face(size=30), moved], bearings)
        result = tracker.speaker(900_000_000, 1_150_000_000)
        self.assertEqual((result['strategy'], result['speaker']['track_id']), ('mouth_motion', second_id))
        # A window after the movement has no mouth evidence.
        result = tracker.speaker(1_100_000_000, 1_150_000_000)
        self.assertEqual((result['strategy'], result['speaker']['track_id']), ('looking_at_orion', first_id))
        tracks[0].face = face(size=30, frontal=False)
        result = tracker.speaker(1_100_000_000, 1_150_000_000)
        self.assertEqual((result['strategy'], result['speaker']['track_id']), ('nearest', second_id))
        self.assertIsNone(tracker.speaker(0, 3_000_000_000)['speaker'])
        with self.assertRaises(ValueError):
            tracker.speaker(4_000_000_000, 3_000_000_000)

    def test_speaker_ignores_translation_scale_and_image_rotation_and_clears_privately(self):
        tracker = Tracker()
        original = face()
        tracker.update(1_000_000_000, [original], [{'azimuth': .1, 'elevation': 0}])
        rotated = face()
        matrix = np.array([[math.cos(.3), -math.sin(.3)], [math.sin(.3), math.cos(.3)]])
        rotated.landmarks = (np.asarray(original.landmarks) @ matrix.T * 1.4 + [10, 20]).tolist()
        tracker.update(1_100_000_000, [rotated], [{'azimuth': .1, 'elevation': 0}])
        self.assertLess(tracker.tracks[0].mouth_motion(0, 1_100_000_000), 1e-12)
        self.assertNotEqual(tracker.speaker(0, 1_100_000_000)['strategy'], 'mouth_motion')
        tracker.clear()
        self.assertIsNone(tracker.speaker(0, 1_100_000_000)['speaker'])

    def test_ids_survive_head_turn_using_capture_time_socket_contract(self):
        with tempfile.TemporaryDirectory() as directory:
            server = FakeRuntime(Path(directory) / 'runtime.sock')
            thread = Thread(target=server.serve_forever, kwargs={'poll_interval': .01}, daemon=True)
            thread.start()
            try:
                client, tracker, calibration = GazeClient(server.server_address), Tracker(), intrinsics()
                ids = None
                for index, yaw in enumerate(np.linspace(-.35, .35, 15)):
                    stamp = 1_000_000_000 + index * 100_000_000
                    server.head_at_capture[stamp] = yaw
                    world = [-.15, .35]
                    if index % 2:
                        world.reverse()  # Detector ordering may change.
                    points = np.array([[math.tan(yaw - az), 0, 1] for az in world])
                    pixels, _ = cv2.fisheye.projectPoints(points.reshape(-1, 1, 3), np.zeros(3), np.zeros(3), calibration.matrix, calibration.distortion)
                    faces = [face(pixel) for pixel in pixels.reshape(-1, 2)]
                    tracks = tracker.update(stamp, faces, client.bearings(stamp, calibration.rays([f.centre for f in faces])))
                    by_bearing = [track.id for track in sorted(tracks, key=lambda t: t.state[0])]
                    ids = ids or by_bearing
                    self.assertEqual(by_bearing, ids)
                    np.testing.assert_allclose(sorted(t.state[0] for t in tracks), [-.15, .35], atol=1e-6)
                self.assertTrue(all(command.startswith('camera-rays ') for command in server.commands))
            finally:
                server.shutdown(); server.server_close(); thread.join(1)

    def test_selection_age_limits_and_normalized_studio_boxes(self):
        tracker = Tracker()
        tracks = tracker.update(100, [face(size=30), face(size=80, frontal=False)],
            [{'azimuth': -.4, 'elevation': 0}, {'azimuth': .4, 'elevation': 0}])
        self.assertEqual(choose_track(tracks).id, tracks[0].id)
        tracks[0].face = face(size=30, frontal=False)
        self.assertEqual(choose_track(tracks).id, tracks[1].id)
        observations = tracker.observations(200)
        self.assertEqual(observations[0]['class'], 'face')
        self.assertTrue(all(0 <= value <= 1 for value in observations[0]['box']))
        self.assertEqual(tracker.observations(1_000_000_101), [])
        with self.assertRaises(ValueError):
            tracker.update(100, [], [])


class PipelineTests(unittest.TestCase):
    def test_voice_throttle_covers_confirmation_through_playback(self):
        client = GazeClient()
        for phase in ('listening', 'thinking', 'processing', 'speaking'):
            with patch.object(client, 'command', return_value={'voice': {'session': 'owned', 'confirmed': True, 'phase': phase}}):
                self.assertTrue(client.voice_active())
        for voice in ({'session': 'owned', 'confirmed': False, 'phase': 'listening'},
                      {'session': None, 'confirmed': True, 'phase': 'speaking'},
                      {'session': 'owned', 'confirmed': True, 'phase': 'guard'}):
            with patch.object(client, 'command', return_value={'voice': voice}):
                self.assertFalse(client.voice_active())

    def test_slow_detector_replaces_pending_frames_and_privacy_discards_inflight_result(self):
        entered, release = Event(), Event()
        seen = []
        class Slow:
            def detect(self, buffer):
                seen.append(buffer)
                if len(seen) == 1:
                    entered.set()
                    release.wait(1)
                return [face()]
        client = MagicMock()
        client.voice_active.return_value = False
        client.bearings.return_value = [{'azimuth': 0, 'elevation': 0}]
        pipeline = FacePipeline(Slow(), intrinsics(), client, boot_clock=lambda: 500)
        self.addCleanup(pipeline.close)
        pipeline.reset(True); pipeline.start(); pipeline.submit(100, 1)
        self.assertTrue(entered.wait(1))
        for value in range(2, 21):
            pipeline.submit(100 + value, value)
        self.assertEqual(pipeline.dropped, 18)
        release.set()
        eventually(lambda: len(seen) == 2)
        self.assertEqual(seen, [1, 20])
        eventually(lambda: client.target.call_count == 2)
        pipeline.reset(False)
        self.assertEqual(pipeline.observations(), [])
        pipeline.submit(200, 99)
        time.sleep(.12)
        self.assertEqual(seen, [1, 20])
        self.assertGreater(client.clear.call_count, 0)

        entered.clear(); release.clear(); seen.clear(); client.target.reset_mock()
        pipeline.reset(True); pipeline.submit(300, 1)
        self.assertTrue(entered.wait(1))
        pipeline.reset(False)
        release.set()
        eventually(lambda: pipeline.inferences == 3)
        self.assertFalse(client.target.called)

    def test_inference_rates_and_voice_status_fail_closed(self):
        for active, minimum_period in [(False, .1), (True, 1/3), (OSError('unavailable'), 1/3)]:
            starts = []
            class Detector:
                def detect(self, buffer):
                    starts.append(time.monotonic())
                    return []
            client = MagicMock()
            if isinstance(active, Exception):
                client.voice_active.side_effect = active
            else:
                client.voice_active.return_value = active
            pipeline = FacePipeline(Detector(), intrinsics(), client)
            try:
                pipeline.reset(True); pipeline.start()
                deadline = time.monotonic() + .75
                while time.monotonic() < deadline:
                    pipeline.submit(boottime_ns(), 1)
                    time.sleep(.01)
                self.assertGreaterEqual(len(starts), 2)
                self.assertTrue(all(b - a >= minimum_period - .002 for a, b in zip(starts, starts[1:])))
                self.assertEqual(pipeline.rate_hz, 10 if active is False else 3)
            finally:
                pipeline.close()


if __name__ == '__main__':
    unittest.main()
