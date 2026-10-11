"""Tracking mode gates inference and attention while preserving camera viewing."""
import http.client
import json
from pathlib import Path
import socket
import tempfile
from threading import Event, Thread
import time
import unittest
from unittest.mock import MagicMock

import numpy as np

from orion_vision.intrinsics import SIZE
from orion_vision.pipeline import FacePipeline
from orion_vision.service import Vision, VisionServer, tracking_mode
from test_capture_control import QuietSource
from test_tracking import eventually, face, intrinsics


class TrackingPreferenceTests(unittest.TestCase):
    def test_missing_defaults_on_and_invalid_settings_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'tracking.json'
            self.assertEqual(tracking_mode(path), 'faces')
            for value in ('bad json', '[]', '{}', 'null', '{"tracking_mode":1}',
                          '{"tracking_mode":"bodies"}', '{"tracking_mode":"off"}'):
                path.write_text(value)
                self.assertEqual(tracking_mode(path), 'off', value)
            path.unlink(); path.mkdir()
            self.assertEqual(tracking_mode(path), 'off')

    def test_off_stops_inference_and_boxes_but_keeps_feed_then_resumes_with_fresh_tracks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = QuietSource()
            now = [1_000_000_000]
            detector, client = MagicMock(), MagicMock()
            detector.detect.return_value = [face()]
            client.voice_active.return_value = False
            client.bearings.return_value = [{'azimuth': 0, 'elevation': 0}]
            pipeline = FacePipeline(detector, intrinsics(), client, boot_clock=lambda: now[0])
            vision = Vision(source, root / 'camera.json', pipeline=pipeline)
            server = VisionServer(root / 'vision.sock', vision)
            thread = Thread(target=server.serve_forever, kwargs={'poll_interval': .01}, daemon=True)
            thread.start(); vision.start()
            def request(path, status=200):
                peer = http.client.HTTPConnection('localhost')
                peer.sock = socket.socket(socket.AF_UNIX)
                peer.sock.connect(str(root / 'vision.sock'))
                try:
                    peer.request('GET', path)
                    response = peer.getresponse()
                    self.assertEqual(response.status, status)
                    return json.loads(response.read())
                finally:
                    peer.close()
            try:
                eventually(lambda: source.starts == 1)
                source.emit(now[0])
                eventually(lambda: client.target.called)
                first_id = request('/observations')[0]['id']
                calls = detector.detect.call_count
                vision.tracking_file.write_text('{"tracking_mode":"off"}')
                eventually(lambda: not pipeline.enabled)
                client.clear.assert_called_once()
                with vision.viewing():
                    for _ in range(5):
                        now[0] += 100_000_000
                        source.emit(now[0])
                    self.assertIsNotNone(vision.jpeg())
                    self.assertEqual(request('/detections')['detections'], [])
                    self.assertEqual(request('/observations'), [])
                    self.assertEqual(request('/speaker?since_ns=0', 503)['error']['code'], 'tracking_off')
                    # Off streams live captures rather than a frozen inference preview.
                    peer = http.client.HTTPConnection('localhost', timeout=2)
                    peer.sock = socket.socket(socket.AF_UNIX)
                    peer.sock.settimeout(2)
                    peer.sock.connect(str(root / 'vision.sock'))
                    try:
                        peer.request('GET', '/stream')
                        response = peer.getresponse()
                        self.assertEqual(response.status, 200)
                        def next_frame():
                            self.assertEqual(response.readline(), b'--orion-frame\r\n')
                            headers = {}
                            while (line := response.readline()) != b'\r\n':
                                key, value = line.decode().split(':', 1)
                                headers[key.lower()] = value.strip()
                            self.assertTrue(response.read(int(headers['content-length'])).startswith(b'\xff\xd8'))
                            self.assertEqual(response.read(2), b'\r\n')
                            return int(headers['x-capture-boottime-ns'])
                        self.assertEqual(next_frame(), now[0])
                        time.sleep(.08)  # Shared JPEG throttle is 15 Hz.
                        now[0] += 100_000_000
                        source.emit(now[0])
                        self.assertEqual(next_frame(), now[0])
                    finally:
                        peer.close()
                time.sleep(.15)
                self.assertEqual(detector.detect.call_count, calls)
                self.assertFalse(source.wants_lores())
                self.assertEqual(source.starts, 1)
                self.assertEqual(request('/health')['tracking_mode'], 'off')
                vision.tracking_file.write_text('{"tracking_mode":"hands"}')
                eventually(lambda: vision.tracking_mode == 'hands')
                self.assertEqual(request('/speaker?since_ns=0', 503)['error']['code'], 'tracking_off')
                vision.tracking_file.write_text('{"tracking_mode":"faces"}')
                eventually(lambda: pipeline.enabled and pipeline.mode == "faces")
                now[0] += 100_000_000
                source.emit(now[0])
                eventually(lambda: len(pipeline.observations()) == 1)
                self.assertNotEqual(request('/observations')[0]['id'], first_id)
                self.assertGreater(detector.detect.call_count, calls)
                self.assertEqual(request('/health')['tracking_mode'], 'faces')
                client.clear.assert_called_once()
            finally:
                vision.close(); server.shutdown(); server.server_close(); thread.join(1)

    def test_off_discards_an_inference_already_in_flight(self):
        started, release = Event(), Event()
        detector, client = MagicMock(), MagicMock()
        def detect(buffer):
            started.set()
            release.wait(2)
            return [face()]
        detector.detect.side_effect = detect
        client.voice_active.return_value = False
        client.bearings.return_value = [{'azimuth': 0, 'elevation': 0}]
        pipeline = FacePipeline(detector, intrinsics(), client, boot_clock=lambda: 1_000_000_000)
        pipeline.start(); pipeline.reset(True)
        try:
            buffer = np.zeros((SIZE[1], SIZE[0], 3), np.uint8)
            pipeline.submit(1_000_000_000, buffer)
            self.assertTrue(started.wait(1))
            pipeline.submit(1_100_000_000, buffer)
            pipeline.reset(False)
            release.set()
            time.sleep(.15)
            self.assertEqual(detector.detect.call_count, 1)
            client.target.assert_not_called()
            self.assertEqual(pipeline.observations(), [])
            self.assertIsNone(pipeline.preview())
        finally:
            release.set(); pipeline.close()
