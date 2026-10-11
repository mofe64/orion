"""Synthetic frames exercise the real Unix HTTP server without camera hardware."""
import http.client
import json
from pathlib import Path
import socket
import tempfile
from threading import Thread
import time
import unittest

import numpy as np
import simplejpeg

from orion_vision.service import Vision, VisionServer, camera_enabled, exposure_midpoint
from orion_vision.sources import Capture, Picamera2Source, ReplaySource
from unittest.mock import MagicMock, patch
import sys


def eventually(predicate, timeout=1):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(.01)
    raise AssertionError("Timed out waiting for condition")


class SocketTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        directory = Path(self.tmp.name)
        # Only synthetic input images are written. Output JPEGs remain in memory.
        image = np.zeros((720, 1280, 3), dtype=np.uint8)
        image[:, :, 0] = 240
        (directory / "test.jpg").write_bytes(simplejpeg.encode_jpeg(image))
        self.preference = directory / "camera.json"
        self.preference.write_text('{"camera_enabled":true}')
        self.source = ReplaySource(directory)
        self.vision = Vision(self.source, self.preference)
        self.path = directory / "vision.sock"
        self.server = VisionServer(self.path, self.vision)
        self.thread = Thread(target=self.server.serve_forever, kwargs={"poll_interval": .01}, daemon=True)
        self.thread.start()
        self.vision.start()
        self.addCleanup(self.shutdown)
        eventually(lambda: self.vision.last_capture > 0)

    def shutdown(self):
        self.vision.close()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(1)

    def request(self, path):
        client = http.client.HTTPConnection("localhost", timeout=2)
        client.sock = socket.socket(socket.AF_UNIX)
        client.sock.settimeout(2)
        client.sock.connect(str(self.path))
        client.request("GET", path)
        return client, client.getresponse()

    def part(self, response):
        self.assertEqual(response.readline(), b"--orion-frame\r\n")
        headers = {}
        while (line := response.readline()) != b"\r\n":
            self.assertTrue(line)
            key, value = line.decode().strip().split(": ", 1)
            headers[key] = value
        jpeg = response.read(int(headers["Content-Length"]))
        self.assertEqual(response.read(2), b"\r\n")
        self.assertTrue(simplejpeg.is_jpeg(jpeg))
        self.assertEqual(simplejpeg.decode_jpeg(jpeg).shape, (360, 640, 3))
        self.assertGreater(int(headers["X-Capture-Boottime-Ns"]), 0)
        return jpeg

    def test_replay_snapshot_health_and_observations(self):
        self.assertEqual(self.vision.encodes, 0)
        self.assertIsNone(self.vision.latest)
        for path in ("/health", "/observations"):
            client, response = self.request(path)
            self.assertEqual(response.status, 200)
            value = json.loads(response.read())
            self.assertEqual(value, [] if path == "/observations" else value)
            if path == "/health":
                self.assertEqual(value["clock"], "CLOCK_BOOTTIME")
                self.assertGreater(value["capture_boottime_ns"], 0)
            client.close()
        self.assertEqual(self.vision.encodes, 0)
        client, response = self.request("/frame.jpg")
        self.assertEqual(response.status, 200)
        rgb = simplejpeg.decode_jpeg(response.read())
        self.assertEqual(rgb.shape, (360, 640, 3))
        self.assertGreater(rgb[0, 0, 0], 230)
        self.assertLess(rgb[0, 0, 2], 10)
        client.close()

    def test_stream_valid_parts_rate_and_encoding_stops_after_disconnect(self):
        client, response = self.request("/stream")
        self.assertEqual(response.status, 200)
        self.part(response)
        time.sleep(.22)
        self.part(response)
        self.assertLessEqual(self.vision.encodes, 5)
        response.close()
        client.close()
        eventually(lambda: self.vision.clients == 0)
        count = self.vision.encodes
        time.sleep(.15)
        self.assertEqual(self.vision.encodes, count)

    def test_off_closes_camera_drops_buffers_and_all_endpoints_return_503(self):
        client, response = self.request("/stream")
        self.part(response)
        started = time.monotonic()
        self.preference.write_text('{"camera_enabled":false}')
        eventually(lambda: not self.vision.enabled and self.source.thread is None)
        self.assertLess(time.monotonic() - started, 1)
        self.assertIsNone(self.vision.latest)
        self.assertIsNone(self.vision.cached)
        eventually(lambda: self.vision.clients == 0)
        response.close(); client.close()
        for path in ("/health", "/observations", "/frame.jpg", "/stream", "/unknown"):
            with self.subTest(path=path):
                client, response = self.request(path)
                self.assertEqual(response.status, 503)
                self.assertEqual(json.loads(response.read())["error"]["code"], "camera_off")
                client.close()
        self.preference.write_text('{"camera_enabled":true}')
        eventually(lambda: self.vision.last_capture > 0)

    def test_idle_snapshot_requests_rgb_then_returns_to_metadata_only(self):
        clock = MagicMock(return_value=10)
        self.vision.clock = clock
        self.assertFalse(self.vision.wants_rgb())
        client, response = self.request('/frame.jpg')
        self.assertEqual(response.status, 200)
        first_stamp = int(response.headers['X-Capture-Boottime-Ns'])
        self.assertTrue(simplejpeg.is_jpeg(response.read()))
        client.close()
        self.assertEqual(self.vision.snapshot_until, 12)
        self.assertTrue(self.vision.wants_rgb())
        clock.return_value = 12
        eventually(lambda: self.vision.latest is None)
        self.assertFalse(self.vision.wants_rgb())
        self.assertIsNone(self.vision.cached)
        client, response = self.request('/frame.jpg')
        self.assertEqual(response.status, 200)
        self.assertGreater(int(response.headers['X-Capture-Boottime-Ns']), first_stamp)
        self.assertTrue(simplejpeg.is_jpeg(response.read()))
        client.close()

    def test_first_stream_from_idle_requests_rgb_and_releases_demand_on_disconnect(self):
        self.assertFalse(self.vision.wants_rgb())
        client, response = self.request('/stream')
        self.part(response)
        self.assertTrue(self.vision.wants_rgb())
        response.close(); client.close()
        eventually(lambda: self.vision.clients == 0 and self.vision.latest is None)
        self.assertFalse(self.vision.wants_rgb())

    def test_stream_wait_timeout_does_not_leak_rgb_demand(self):
        self.source.close()
        client, response = self.request('/stream')
        self.assertEqual(response.status, 503)
        self.assertEqual(json.loads(response.read())['error']['code'], 'no_frame')
        client.close()
        self.assertEqual(self.vision.clients, 0)
        self.assertFalse(self.vision.wants_rgb())

    def test_shared_encode_cache_for_two_viewers(self):
        client1, response1 = self.request("/stream")
        self.part(response1)
        client2, response2 = self.request("/stream")
        self.part(response2)
        self.assertEqual(self.vision.encodes, 1)
        response1.close(); client1.close()
        response2.close(); client2.close()

    def test_tracking_observations_and_privacy_use_the_same_source_lifecycle(self):
        from orion_vision.calibration import Intrinsics
        from orion_vision.detect_faces import Face
        from orion_vision.pipeline import FacePipeline
        detector, client = MagicMock(), MagicMock()
        detector.detect.return_value = [Face([100, 80, 40, 40],
            [[110, 90], [130, 90], [120, 100], [113, 110], [127, 110]], .95)]
        client.voice_active.return_value = False
        client.bearings.return_value = [{"azimuth": .2, "elevation": .1}]
        pipeline = FacePipeline(detector, Intrinsics([[125, 0, 160], [0, 125, 120], [0, 0, 1]], [0]*4, .1), client)
        self.vision.close()
        self.vision = Vision(self.source, self.preference, pipeline=pipeline)
        self.server.vision = self.vision
        self.vision.start()
        eventually(lambda: client.target.called)
        connection, response = self.request('/observations')
        observations = json.loads(response.read())
        connection.close()
        self.assertEqual(response.status, 200)
        self.assertAlmostEqual(observations[0]['azimuth'], .2, places=12)
        np.testing.assert_allclose(observations[0]['box'], [100/320, 80/240, 40/320, 40/240])
        self.assertIsNone(self.vision.latest)  # Inference does not demand main RGB.
        connection, response = self.request('/speaker?since_ns=0')
        speaker = json.loads(response.read())
        connection.close()
        self.assertEqual(response.status, 200)
        self.assertEqual(speaker['speaker']['track_id'], observations[0]['track_id'])
        self.assertEqual(speaker['strategy'], 'looking_at_orion')
        for path in ('/speaker', '/speaker?since_ns=-1', '/speaker?since_ns=bad', '/speaker?since_ns=0&since_ns=1'):
            connection, response = self.request(path)
            self.assertEqual(response.status, 400)
            response.read(); connection.close()
        self.preference.write_text('{"camera_enabled":false}')
        eventually(lambda: not self.vision.enabled and self.source.thread is None)
        eventually(lambda: not pipeline.enabled)
        self.assertEqual(pipeline.observations(), [])
        self.assertTrue(client.clear.called)
        connection, response = self.request('/speaker?since_ns=0')
        self.assertEqual(response.status, 503)
        self.assertEqual(json.loads(response.read())['error']['code'], 'camera_off')
        connection.close()


class MetadataTests(unittest.TestCase):
    def test_mid_exposure_units_and_invalid_metadata(self):
        self.assertEqual(exposure_midpoint({"SensorTimestamp": 1000000, "ExposureTime": 123}), 1061500)
        for metadata in ({}, {"SensorTimestamp": 1}, {"SensorTimestamp": None, "ExposureTime": 1},
                         {"SensorTimestamp": True, "ExposureTime": 1}, {"SensorTimestamp": 1, "ExposureTime": -1}):
            self.assertIsNone(exposure_midpoint(metadata))

    def test_non_increasing_sensor_or_midpoint_and_stale_generation_drop(self):
        vision = Vision(MagicMock(), Path("/nonexistent-preference"))
        vision.enabled = True
        rgb = np.zeros((2, 2, 3), dtype=np.uint8)
        vision.publish(Capture(rgb, {"SensorTimestamp": 10000, "ExposureTime": 10}), 0)
        latest = vision.latest
        for metadata in ({}, {"SensorTimestamp": 10000, "ExposureTime": 20},
                         {"SensorTimestamp": 9999, "ExposureTime": 30},
                         {"SensorTimestamp": 10001, "ExposureTime": 0}):
            vision.publish(Capture(rgb, metadata), 0)
            self.assertIs(vision.latest, latest)
        self.assertEqual(vision.dropped, 4)
        vision.publish(Capture(rgb, {"SensorTimestamp": 20000, "ExposureTime": 1}), 99)
        self.assertIs(vision.latest, latest)

    def test_invalid_preference_fails_closed_and_missing_uses_enabled_default(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "camera.json"
            self.assertTrue(camera_enabled(path))
            for text in ('garbage', '{}', '{"camera_enabled":"true"}', '{"camera_enabled":false}'):
                path.write_text(text)
                self.assertFalse(camera_enabled(path))

    def test_picamera_callback_rgb_format_and_no_preview(self):
        camera = MagicMock()
        camera_module = MagicMock()
        camera_module.Picamera2.return_value = camera
        with patch.dict(sys.modules, picamera2=camera_module, libcamera=MagicMock()):
            source = Picamera2Source(0, False, False)
            publish = MagicMock()
            source.start(publish, lambda: True)
            options = camera.create_video_configuration.call_args.kwargs
            self.assertEqual(options["main"]["format"], "BGR888")
            self.assertFalse(options["queue"])
            request = MagicMock()
            request.make_array.return_value = np.zeros((2, 2, 3), dtype=np.uint8)
            request.get_metadata.return_value = {"SensorTimestamp": 4, "ExposureTime": 0}
            camera.post_callback(request)
            request.make_array.assert_called_once_with('main')
            self.assertIs(publish.call_args.args[0].rgb, request.make_array.return_value)
            camera.start.assert_called_once_with(show_preview=False)
            source.close()
            camera.stop.assert_called_once()
            camera.close.assert_called_once()

    def test_picamera_idle_callback_only_publishes_metadata_until_viewing_demand(self):
        camera = MagicMock()
        camera_module = MagicMock()
        camera_module.Picamera2.return_value = camera
        clock = MagicMock(return_value=10)
        with patch.dict(sys.modules, picamera2=camera_module, libcamera=MagicMock()):
            source = Picamera2Source()
            self.addCleanup(source.close)
            vision = Vision(source, Path('/nonexistent-preference'), clock=clock)
            vision.enabled = True
            source.start(lambda capture: vision.publish(capture, 0), vision.wants_rgb)
            request = MagicMock()
            request.make_array.return_value = np.zeros((2, 2, 3), dtype=np.uint8)

            def capture(stamp):
                request.get_metadata.return_value = {'SensorTimestamp': stamp, 'ExposureTime': 0}
                camera.post_callback(request)

            capture(1)
            request.make_array.assert_not_called()
            self.assertIsNone(vision.latest)
            self.assertEqual(vision.status()[1]['capture_boottime_ns'], 1)
            with vision.viewing():
                capture(2)
                request.make_array.assert_called_once_with('main')
                self.assertIs(vision.latest[1], request.make_array.return_value)
            request.make_array.reset_mock()
            capture(3)
            request.make_array.assert_not_called()
            with vision.viewing(snapshot=True):
                capture(4)
            request.make_array.reset_mock()
            clock.return_value = 11.99
            capture(5)
            request.make_array.assert_called_once_with('main')
            request.make_array.reset_mock()
            clock.return_value = 12
            capture(6)
            request.make_array.assert_not_called()
            self.assertIsNone(vision.latest)
            self.assertEqual(vision.status()[1]['capture_boottime_ns'], 6)
            self.assertEqual(vision.dropped, 0)


if __name__ == "__main__":
    unittest.main()
