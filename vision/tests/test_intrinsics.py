import math
import unittest
from unittest.mock import MagicMock, patch

import numpy as np

from orion_vision.intrinsics import SIZE, Intrinsics
from orion_vision.sources import Picamera2Source


class BuiltinIntrinsicsTests(unittest.TestCase):
    def test_centred_pixel_is_the_optical_axis(self):
        model = Intrinsics.camera_module_3_wide()
        np.testing.assert_allclose(model.rays([[SIZE[0] / 2, SIZE[1] / 2]]), [[0, 0, 1]], atol=1e-12)

    def test_nominal_intrinsics_have_square_pixels_and_centred_principal_point(self):
        matrix = Intrinsics.camera_module_3_wide().matrix
        self.assertAlmostEqual(matrix[0, 0] / matrix[1, 1], 1, delta=.01)
        np.testing.assert_allclose(matrix[:2, 2], np.asarray(SIZE) / 2, atol=1e-12)

    def test_horizontal_edges_match_the_wide_lens_datasheet(self):
        model = Intrinsics.camera_module_3_wide()
        left, right = model.rays([[0, SIZE[1] / 2], [SIZE[0] - 1, SIZE[1] / 2]])
        for ray in (left, right):
            angle = math.degrees(math.atan2(abs(ray[0]), ray[2]))
            self.assertLess(abs(angle - 102 / 2), 3)
        self.assertLess(left[0], 0)
        self.assertGreater(right[0], 0)

    def test_vertical_edges_account_for_the_lores_aspect_ratio(self):
        model = Intrinsics.camera_module_3_wide()
        for ray in model.rays([[SIZE[0] / 2, 0], [SIZE[0] / 2, SIZE[1] - 1]]):
            angle = math.degrees(math.atan2(abs(ray[1]), ray[2]))
            self.assertLess(abs(angle - 67 / 2), 3)

    def test_source_requests_full_view_and_records_the_configured_stride(self):
        camera, module = MagicMock(), MagicMock()
        stride = SIZE[0] + 64  # Exercise negotiated padding even at an aligned width.
        module.Picamera2.return_value = camera
        camera.camera_configuration.return_value = {
            'sensor': {'output_size': (2304, 1296), 'bit_depth': 10},
            'lores': {'size': SIZE, 'format': 'YUV420', 'stride': stride}}
        with patch.dict('sys.modules', picamera2=module, libcamera=MagicMock()):
            source = Picamera2Source()
            self.addCleanup(source.close)
            source.start(MagicMock(), lambda: False, lambda: True)
            config = camera.create_video_configuration.call_args.kwargs
            self.assertEqual(config['sensor'], {'output_size': [2304, 1296], 'bit_depth': 10})
            self.assertEqual(config['controls']['ScalerCrop'], (0, 0, 4608, 2592))
            self.assertEqual(config['lores'], {'size': SIZE, 'format': 'YUV420', 'preserve_ar': False})
            self.assertEqual(source.expected_geometry['lores_stride'], stride)
