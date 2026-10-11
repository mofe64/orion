"""Capture recovery follows publications, independently of inference and viewing."""
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import Mock

import numpy as np

from orion_vision.intrinsics import SIZE
from orion_vision.service import Vision
from orion_vision.sources import Capture
from test_tracking import eventually


class QuietSource:
    def __init__(self):
        self.starts = self.closes = 0

    def start(self, publish, wants_rgb, wants_lores=None):
        self.starts += 1
        self.publish, self.wants_rgb, self.wants_lores = publish, wants_rgb, wants_lores

    def emit(self, stamp):
        rgb = np.zeros((4, 4, 3), np.uint8) if self.wants_rgb() else None
        lores = np.zeros((SIZE[1], SIZE[0], 3), np.uint8) if self.wants_lores and self.wants_lores() else None
        self.publish(Capture(rgb, {'SensorTimestamp': stamp, 'ExposureTime': 0}, lores))

    def close(self):
        self.closes += 1


class CaptureWatchdogTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.preference = Path(temporary.name) / 'camera.json'
        self.now = 0.
        self.source = QuietSource()
        self.vision = Vision(self.source, self.preference, clock=lambda: self.now)
        self.addCleanup(self.vision.close)

    def test_stalled_source_reopens_after_grace_and_drops_old_generation(self):
        self.vision.start()
        eventually(lambda: self.source.starts == 1)
        self.source.emit(1)
        old_publish = self.source.publish
        self.now = 3.
        time.sleep(.15)
        self.assertEqual(self.source.starts, 1)  # Startup grace is strictly over 3 s.
        with self.assertLogs('orion_vision.service', level='WARNING') as logs:
            self.now = 3.1
            eventually(lambda: self.source.starts == 2)
        self.assertEqual(len(logs.records), 1)
        self.assertEqual(self.source.closes, 1)
        self.source.emit(2)
        old_publish(Capture(None, {'SensorTimestamp': 99, 'ExposureTime': 0}))
        self.assertEqual(self.vision.last_capture, 2)
        health = self.vision.status()[1]
        self.assertEqual(health['camera_restarts'], 1)
        self.assertEqual(health['capture_boottime_ns'], 2)
        self.preference.write_text('{"camera_enabled":false}')
        eventually(lambda: not self.vision.enabled)
        self.assertEqual(self.vision.status()[1]['camera_restarts'], 1)
        self.preference.write_text('{"camera_enabled":true}')
        eventually(lambda: self.source.starts == 3)
        self.source.emit(3)
        self.assertEqual(self.vision.status()[1]['camera_restarts'], 1)

    def test_progress_and_camera_privacy_off_do_not_trigger_restarts(self):
        self.vision.start()
        eventually(lambda: self.source.starts == 1)
        with self.vision.changed:
            self.now = 4.
            self.source.emit(1)
        self.now = 5.9
        time.sleep(.15)
        self.assertEqual(self.vision.camera_restarts, 0)
        self.preference.write_text('{"camera_enabled":false}')
        self.now = 100.
        eventually(lambda: not self.vision.enabled)
        self.assertEqual(self.vision.camera_restarts, 0)
        self.assertEqual(self.source.starts, 1)

    def test_hands_watchdog_reopen_does_not_send_off_but_privacy_still_does(self):
        from orion_vision.pipeline import FacePipeline
        from test_objects import intrinsics
        client = Mock()
        pipeline = FacePipeline(Mock(), intrinsics(), client)
        self.vision.pipeline = pipeline
        self.vision.tracking_file.write_text('{"tracking_mode":"hands"}')
        self.vision.start()
        eventually(lambda: self.source.starts == 1)
        self.source.emit(1)
        client.reset_mock()
        with self.assertLogs('orion_vision.service', level='WARNING'):
            self.now = 3.1
            eventually(lambda: self.source.starts == 2)
        self.assertEqual(self.vision.camera_restarts, 1)
        client.clear_surroundings.assert_not_called()
        self.assertFalse(any(call.args == ('gaze hands off',) for call in client.command.call_args_list))
        self.assertEqual(pipeline.mode, 'hands')
        # Reopening does not fabricate fresh hand evidence or keep old pixels.
        self.assertIsNone(pipeline.hand_point)
        self.assertIsNone(pipeline.pending)
        self.preference.write_text('{"camera_enabled":false}')
        eventually(lambda: not self.vision.enabled and client.clear_surroundings.called)
        eventually(lambda: any(call.args == ('gaze hands off',) for call in client.command.call_args_list))
