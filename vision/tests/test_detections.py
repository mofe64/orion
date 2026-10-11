"""Preview boxes are capture evidence, not retained gaze predictions."""
import json
from threading import Event
import unittest
from unittest.mock import MagicMock

import numpy as np

from orion_vision.pipeline import FacePipeline
from orion_vision.intrinsics import SIZE
from orion_vision.sources import Capture
import test_service as service_tests
from test_tracking import eventually, face, intrinsics


class PreviewTests(unittest.TestCase):
    def setUp(self):
        self.now = 12345678901234567
        self.detector, self.client = MagicMock(), MagicMock()
        self.detector.detect.return_value = [face(size=30), face((240, 120), size=60, frontal=False)]
        self.client.voice_active.return_value = False
        self.client.bearings.return_value = [{'azimuth': -.2, 'elevation': 0}, {'azimuth': .4, 'elevation': 0}]
        self.pipeline = FacePipeline(self.detector, intrinsics(), self.client, boot_clock=lambda: self.now)
        self.pipeline.reset(True); self.pipeline.start()
        self.addCleanup(self.pipeline.close)

    def test_preview_pairs_pixels_and_fresh_detections_with_exact_timestamp(self):
        rgb = np.zeros((720, 1280, 3), np.uint8)
        self.pipeline.submit(self.now, 1, rgb=rgb)
        eventually(lambda: self.pipeline.preview() is not None)
        stamp, pixels, detections = self.pipeline.preview()
        self.assertEqual(stamp, self.now)
        self.assertIs(pixels, rgb)
        self.assertEqual([box['track_id'] for box in detections], [1, 2])
        self.assertEqual([box['chosen_target'] for box in detections], [True, False])
        np.testing.assert_allclose(detections[0]['box'],
            [145 / SIZE[0], 105 / SIZE[1], 30 / SIZE[0], 30 / SIZE[1]])
        self.assertEqual(detections[0]['score'], .95)
        self.assertEqual(detections[0]['capture_boottime_ns'], str(self.now))
        # Retained tracking evidence must never become a box in another frame.
        self.now += 100_000_000
        self.detector.detect.return_value = []
        self.pipeline.submit(self.now, 2, rgb=rgb)
        eventually(lambda: self.pipeline.preview()[0] == self.now)
        self.assertEqual(self.pipeline.preview()[2], [])
        self.assertEqual(len(self.pipeline.observations()), 2)

    def test_releasing_preview_discards_inflight_rgb_without_cancelling_gaze(self):
        entered, finish = Event(), Event()
        self.addCleanup(finish.set)
        def detect(_):
            entered.set(); finish.wait(1)
            return [face()]
        self.detector.detect.side_effect = detect
        self.client.bearings.return_value = [{'azimuth': 0, 'elevation': 0}]
        self.pipeline.submit(self.now, 1, rgb=np.zeros((720, 1280, 3), np.uint8))
        self.assertTrue(entered.wait(1))
        self.pipeline.clear_preview()
        finish.set()
        eventually(lambda: self.client.target.called)
        self.assertIsNone(self.pipeline.preview())
        self.assertEqual(len(self.pipeline.observations()), 1)

    def test_privacy_and_detector_failure_purge_preview(self):
        self.pipeline.submit(self.now, 1, rgb=np.zeros((720, 1280, 3), np.uint8))
        eventually(lambda: self.pipeline.preview() is not None)
        self.detector.detect.side_effect = ValueError('synthetic detector failure')
        self.now += 100_000_000
        self.pipeline.submit(self.now, 2, rgb=np.zeros((720, 1280, 3), np.uint8))
        eventually(lambda: self.pipeline.error is not None)
        self.assertIsNone(self.pipeline.preview())
        self.pipeline.reset(False)
        self.assertIsNone(self.pipeline.preview())


class DetectionSocketTests(unittest.TestCase):
    setUp = service_tests.SocketTests.setUp
    shutdown = service_tests.SocketTests.shutdown
    request = service_tests.SocketTests.request

    def test_real_pipeline_and_http_preview_share_one_capture(self):
        self.source.close()  # Publish deterministic synthetic captures below.
        stamp = self.vision.last_capture + 1
        detector, client = MagicMock(), MagicMock()
        detector.detect.return_value = [face()]
        client.voice_active.return_value = False
        client.bearings.return_value = [{'azimuth': .1, 'elevation': 0}]
        pipeline = FacePipeline(detector, intrinsics(), client, boot_clock=lambda: stamp + 20_000_000)
        self.vision.pipeline = pipeline
        pipeline.reset(True); pipeline.start()
        pixels = np.zeros((720, 1280, 3), np.uint8)
        pixels[:, :, 1] = 240
        with self.vision.viewing():
            self.vision.publish(Capture(pixels, {'SensorTimestamp': stamp, 'ExposureTime': 0},
                                np.zeros((SIZE[1] * 3 // 2, SIZE[0]), np.uint8)), self.vision.generation)
            eventually(lambda: pipeline.preview() is not None)
            peer, image = self.request('/frame.jpg')
            image_stamp = image.headers['X-Capture-Boottime-Ns']
            decoded = service_tests.simplejpeg.decode_jpeg(image.read()); peer.close()
            self.assertGreater(decoded[0, 0, 1], 230)
            peer, metadata = self.request('/detections')
            batch = json.loads(metadata.read()); peer.close()
            self.assertEqual(batch['capture_boottime_ns'], image_stamp)
            self.assertEqual(batch['age_ms'], 20.)
            self.assertEqual(batch['detections'][0]['track_id'], 1)
            self.assertTrue(batch['detections'][0]['chosen_target'])

    def test_detections_require_viewing_and_do_not_acquire_or_extend_a_lease(self):
        client, response = self.request('/detections')
        self.assertEqual(response.status, 503)
        self.assertEqual(json.loads(response.read())['error']['code'], 'camera_not_viewed')
        client.close()
        self.assertEqual(self.vision.clients, 0)
        self.assertEqual(self.vision.snapshot_until, 0)

    def test_detections_proxy_only_the_preview_capture_and_disable_privately(self):
        now = 12345678901234567
        pixels = np.zeros((720, 1280, 3), np.uint8)
        detections = [dict(box=[.1, .2, .3, .4], score=.95, track_id=9,
                           chosen_target=True, capture_boottime_ns=str(now))]
        pipeline = MagicMock()
        pipeline.preview.return_value = (now, pixels, detections)
        pipeline.boot_clock.return_value = now + 100_000_000
        pipeline.status.return_value = {'error': None}
        self.vision.pipeline = pipeline
        with self.vision.viewing():
            frame = self.vision.wait_for_jpeg()
            self.assertEqual(frame[0], now)
            client, response = self.request('/detections')
            self.assertEqual(response.status, 200)
            self.assertEqual(response.headers['Cache-Control'], 'no-store')
            batch = json.loads(response.read()); client.close()
            self.assertEqual(batch, dict(capture_boottime_ns=str(frame[0]), age_ms=100., detections=detections))
            self.preference.write_text('{"camera_enabled":false}')
            client, response = self.request('/detections')
            self.assertEqual(response.status, 503)
            self.assertEqual(json.loads(response.read())['error']['code'], 'camera_off')
            client.close()

    def test_snapshot_lease_expires_without_a_detections_request_extending_it(self):
        clock = MagicMock(return_value=10)
        self.vision.clock = clock
        self.vision.pipeline = MagicMock()
        self.vision.pipeline.status.return_value = {'error': None}
        self.vision.pipeline.preview.return_value = None
        with self.vision.viewing(snapshot=True):
            pass
        client, response = self.request('/detections')
        self.assertEqual(response.status, 200)
        self.assertEqual(json.loads(response.read())['detections'], [])
        client.close()
        self.assertEqual(self.vision.snapshot_until, 12)
        clock.return_value = 12
        client, response = self.request('/detections')
        self.assertEqual(response.status, 503)
        response.read(); client.close()
