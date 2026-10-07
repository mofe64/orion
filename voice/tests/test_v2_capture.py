import unittest
from unittest.mock import patch
import numpy as np
from orion_voice.capture import AlsaPcmCapture
from orion_voice.satellite import StereoCapture


class V2CaptureTests(unittest.TestCase):
    def test_usb_capture_never_applies_hat_mixer_and_discards_startup(self):
        capture = AlsaPcmCapture('plughw:CARD=Array,DEV=0', hardware='v2')
        with patch('orion_voice.capture.subprocess.run') as configure, \
             patch('orion_voice.capture.subprocess.Popen'), patch.object(capture, '_discard_startup') as discard:
            capture.open()
        configure.assert_not_called(); discard.assert_called_once_with(300)

    def test_processed_channel_survives_without_raw_channel_downmix(self):
        for channels in (2, 6):
            capture = StereoCapture('plughw:CARD=Array,DEV=0', hardware='v2', capture_channels=channels, processed_channel=1)
            frames = np.full((320, channels), -2000, dtype='<i2'); frames[:, 1] = 1234
            with patch.object(AlsaPcmCapture, 'read', return_value=frames.tobytes()):
                stereo = np.frombuffer(capture.read(), dtype='<i2').reshape(-1, 2)
            self.assertTrue(np.all(stereo == 1234))
            self.assertEqual(capture.command()[-1], str(channels))
        with self.assertRaises(ValueError): StereoCapture(hardware='v1', capture_channels=6)
        with self.assertRaises(ValueError): StereoCapture(hardware='v2', processed_channel=2)
