import math
import unittest
from unittest.mock import MagicMock, patch

import numpy as np

from orion_vision.intrinsics import Intrinsics
from orion_vision.sources import Picamera2Source


class BuiltinIntrinsicsTests(unittest.TestCase):
    def test_centred_pixel_is_the_optical_axis(self):
        model = Intrinsics.camera_module_3_wide()
        np.testing.assert_allclose(model.rays([[160, 120]]), [[0, 0, 1]], atol=1e-12)

    def test_horizontal_edges_match_the_wide_lens_datasheet(self):
        model = Intrinsics.camera_module_3_wide()
        left, right = model.rays([[0, 120], [319, 120]])
        for ray in (left, right):
            angle = math.degrees(math.atan2(abs(ray[0]), ray[2]))
            self.assertLess(abs(angle - 102 / 2), 3)
        self.assertLess(left[0], 0)
        self.assertGreater(right[0], 0)

    def test_vertical_edges_account_for_the_lores_aspect_ratio(self):
        model = Intrinsics.camera_module_3_wide()
        for ray in model.rays([[160, 0], [160, 239]]):
            angle = math.degrees(math.atan2(abs(ray[1]), ray[2]))
            self.assertLess(abs(angle - 67 / 2), 3)

    def test_source_requests_full_view_and_records_the_configured_stride(self):
        camera, module = MagicMock(), MagicMock()
        module.Picamera2.return_value = camera
        camera.camera_configuration.return_value = {
            'sensor': {'output_size': (2304, 1296), 'bit_depth': 10},
            'lores': {'size': (320, 240), 'format': 'YUV420', 'stride': 384}}
        with patch.dict('sys.modules', picamera2=module, libcamera=MagicMock()):
            source = Picamera2Source()
            self.addCleanup(source.close)
            source.start(MagicMock(), lambda: False, lambda: True)
            config = camera.create_video_configuration.call_args.kwargs
            self.assertEqual(config['sensor'], {'output_size': [2304, 1296], 'bit_depth': 10})
            self.assertEqual(config['controls']['ScalerCrop'], (0, 0, 4608, 2592))
            self.assertEqual(config['lores'], {'size': (320, 240), 'format': 'YUV420', 'preserve_ar': False})
            self.assertEqual(source.expected_geometry['lores_stride'], 384)
