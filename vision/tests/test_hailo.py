"""Fixed model tensors and optional accelerator behavior; no Hailo imports or device."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

import numpy as np

from orion_vision import hailo_backend as hailo
from orion_vision import hailo_models as models
from orion_vision.detector_policy import RATES, SCORES, rates
from orion_vision.pipeline import FacePipeline
from orion_vision.service import accelerator_setting
from test_tracking import eventually, intrinsics
from test_objects import CAMERA
from orion_vision.detect_hands import anchors, decode as cpu_palm_decode


class MultiplexFixture:
    def __init__(self, argument):
        pass

    def detect(self, request):
        return (os.getpid(), request[0])


def scrfd_outputs():
    return {f'{stride}_{channels}': np.zeros((1, 640 // stride, 640 // stride, channels), np.float32)
            for stride in (8, 16, 32) for channels in (8, 2, 20)}


class PostProcessingTests(unittest.TestCase):
    def test_rgb_input_letterbox_and_network_normalization_not_repeated(self):
        pixels = np.full((360, 640, 3), [10, 20, 30], np.uint8)
        image, geometry = hailo.preprocess(pixels, 114)
        self.assertEqual(geometry, (1, 0, 140))
        self.assertEqual(image.shape, (1, 640, 640, 3))
        self.assertEqual(image.dtype, np.uint8)
        np.testing.assert_array_equal(image[0, 140, 0], [10, 20, 30])
        np.testing.assert_array_equal(image[0, 139, 0], [114, 114, 114])

    def test_scrfd_anchor_stride_landmarks_unpadding_and_nms(self):
        outputs = scrfd_outputs()
        # Grid (40, 30), stride 8 -> image point (320, 240).
        outputs['8_8'][0, 30, 40] = [4, 5, 4, 5] * 2
        outputs['8_2'][0, 30, 40] = [.9, .8]  # Duplicate anchor must be suppressed.
        points = np.array([[-2, -1], [2, -1], [0, 0], [-1, 2], [1, 2]])
        outputs['8_20'][0, 30, 40] = np.tile(points.ravel(), 2)
        outputs['16_8'][0, 15, 20, :4] = [2, 2.5, 2, 2.5]
        outputs['16_2'][0, 15, 20, 0] = .7  # Duplicate at another stride.
        outputs['32_8'][0, 10, 10, :4] = 1
        outputs['32_2'][0, 10, 10, 0] = .49  # Independent threshold, not YuNet .75.
        result = hailo.decode_faces(dict(reversed(list(outputs.items()))), (1, 0, 140))
        self.assertEqual(len(result), 1)
        self.assertEqual(result[0].box, [288., 60., 64., 80.])
        np.testing.assert_allclose(result[0].landmarks, points * 8 + [320, 100])
        self.assertTrue(result[0].looking_at_orion)
        self.assertAlmostEqual(result[0].confidence, .9)

    def test_scrfd_rejects_invalid_scores_boxes_and_missing_landmarks(self):
        outputs = scrfd_outputs()
        outputs['8_2'][0, 30, 40] = [np.nan, 2]
        self.assertEqual(hailo.decode_faces(outputs, (1, 0, 140)), [])
        del outputs['8_20']
        with self.assertRaisesRegex(ValueError, 'five landmarks'):
            hailo.decode_faces(outputs, (1, 0, 140))

    def test_nms_keeps_separate_boxes_and_bounds_output(self):
        boxes = np.array([[0, 0, 10, 10], [1, 1, 10, 10], [30, 0, 10, 10]], float)
        self.assertEqual(hailo.nms(boxes, np.array([.9, .8, .7]), .4, 16), [0, 2])
        self.assertEqual(hailo.nms(boxes, np.array([.9, .8, .7]), .4, 1), [0])

    def test_palm_hef_raw_heads_share_cpu_anchor_decode_and_numpy_nms(self):
        boxes = np.zeros((1, 2016, 18), np.float32)
        scores = np.full((1, 2016, 1), -100., np.float32)
        for index, centre, size in ((0, [96, 96], 30), (1, [96, 96], 30), (1152, [40, 70], 15)):
            boxes[0, index, :4] = [*(np.array(centre) - anchors()[index] * 192), size, size]
            scores[0, index, 0] = 10 if index == 0 else 8
        outputs = {'reg16': boxes[:, 1152:].reshape(1, 12, 12, 108),
                   'score8': scores[:, :1152].reshape(1, 24, 24, 2),
                   'reg8': boxes[:, :1152].reshape(1, 24, 24, 36),
                   'score16': scores[:, 1152:].reshape(1, 12, 12, 6)}
        geometry = (.3, 0, 42)
        result = hailo.decode_palms(outputs, geometry)
        self.assertEqual(result, cpu_palm_decode((boxes, scores), geometry))
        self.assertEqual(len(result), 2)
        np.testing.assert_allclose(result[0].box, [270, 130, 100, 100])
        np.testing.assert_allclose(result[1].box, [108.333333, 68.333333, 50, 50], atol=1e-5)
        self.assertEqual(result, hailo.decode_palms({'reg': boxes, 'score': scores}, geometry))
        with self.assertRaisesRegex(ValueError, 'MediaPipe'):
            hailo.decode_palms({'score': scores}, geometry)
        # Supplying UINT8 to the HEF avoids repeating its /255 normalization.
        image, _ = hailo.palm_preprocess(np.full((360, 640, 3), 240, np.uint8), normalise=False)
        self.assertEqual(image.dtype, np.uint8)
        self.assertEqual(int(image[0, 42, 0, 0]), 240)

    def test_hailort_nms_coordinates_class_mapping_score_and_class_filter(self):
        classes = [np.empty((0, 5)) for _ in range(80)]
        # Fixed HailoRT NMS rows: normalized ymin,xmin,ymax,xmax,score.
        classes[41] = np.array([[.3125, .4375, .375, .5625, .9],  # cup
                                [.3125, .4375, .375, .5625, .39],
                                [np.nan, 0, 1, 1, .95], [0, 0, .01, .01, .8]])
        classes[0] = np.array([[0, 0, 1, 1, .8]])  # person, clipped after unpadding
        result = hailo.decode_objects(classes, (1, 0, 140))
        self.assertEqual([item.category for item in result], ['cup', 'person'])
        self.assertEqual(result[0].box, [280., 60., 80., 40.])
        self.assertEqual(result[1].box, [0., 0., 640., 360.])
        self.assertEqual(len(hailo.decode_objects(classes, (1, 0, 140), {'cup'})), 1)
        with self.assertRaises(ValueError):
            hailo.decode_objects(classes[:79], (1, 0, 140))
        classes[0] = np.zeros((1, 6))
        with self.assertRaises(ValueError):
            hailo.decode_objects(classes, (1, 0, 140))


class SelectionTests(unittest.TestCase):
    def test_auto_without_device_uses_cpu_without_loading_hailort(self):
        with patch.object(hailo, 'device_architecture', return_value=None), patch.object(hailo, 'InferenceProcess') as process:
            self.assertIsNone(hailo.select_accelerator())
        process.assert_not_called()

    def test_force_cpu_does_not_probe_the_device(self):
        with patch.object(hailo, 'device_architecture') as probe:
            self.assertIsNone(hailo.select_accelerator('cpu'))
        probe.assert_not_called()

    def test_device_with_missing_or_corrupt_hef_uses_cpu(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(hailo, 'device_architecture', return_value='HAILO8L'), \
             patch.object(hailo, 'InferenceProcess') as process, self.assertLogs(hailo.LOG, 'WARNING'):
            self.assertIsNone(hailo.select_accelerator(directory=directory))
        process.assert_not_called()
        with patch.object(hailo, 'device_architecture', return_value='HAILO8L'), \
             patch.object(hailo, 'checked_model_paths', side_effect=ValueError('SHA mismatch')), self.assertLogs(hailo.LOG, 'WARNING'):
            self.assertIsNone(hailo.select_accelerator())

    def test_8l_files_are_distinct_and_only_one_process_starts(self):
        paths = models.model_paths('HAILO8L')
        self.assertEqual(set(paths), {'faces', 'objects', 'palms'})
        self.assertTrue(all('/hailo8l/' in str(path) for path in paths.values()))
        self.assertNotEqual(paths, models.model_paths('HAILO8'))
        broker = Mock(); broker.detect.return_value = {}
        with patch.object(hailo, 'device_architecture', return_value='HAILO8L'), \
             patch.object(hailo, 'checked_model_paths', return_value=paths) as checked, \
             patch.object(hailo, 'InferenceProcess', return_value=broker) as process:
            self.assertIs(hailo.select_accelerator(), broker)
        checked.assert_called_once_with('HAILO8L', models.MODEL_DIRECTORY)
        process.assert_called_once_with(hailo._HailoNetworks, paths, name='Hailo')

    def test_startup_device_error_uses_cpu(self):
        with patch.object(hailo, 'device_architecture', side_effect=RuntimeError('driver unavailable')), self.assertLogs(hailo.LOG, 'WARNING'):
            self.assertIsNone(hailo.select_accelerator('hailo'))
        with patch.object(hailo, 'device_architecture', return_value='HAILO8L'), \
             patch.object(hailo, 'checked_model_paths', return_value={}), \
             patch.object(hailo, 'InferenceProcess', side_effect=ImportError('hailo_platform')), self.assertLogs(hailo.LOG, 'WARNING'):
            self.assertIsNone(hailo.select_accelerator())

    def test_detector_failure_falls_back_once_and_other_detector_stays_hailo(self):
        for failure in ({'error': 'network failed'}, RuntimeError('child exited')):
            with self.subTest(failure=failure):
                broker = Mock(failures={})
                broker.detect.side_effect = failure if isinstance(failure, Exception) else None
                broker.detect.return_value = failure
                cpu = Mock(); cpu.detect.return_value = ['cpu result']
                factory = Mock(return_value=cpu)
                faces = hailo.HailoDetector('faces', broker, factory)
                objects = hailo.HailoDetector('objects', broker, factory)
                with self.assertLogs(hailo.LOG, 'WARNING') as logs:
                    self.assertEqual(faces.detect('pixels'), ['cpu result'])
                    self.assertEqual(faces.detect('new pixels'), ['cpu result'])
                self.assertEqual(len(logs.output), 1)
                factory.assert_called_once()
                self.assertEqual(faces.backend, 'cpu')
                self.assertEqual(objects.backend, 'hailo')
                self.assertEqual(rates(faces), RATES['cpu'])

    def test_network_startup_failure_and_hailo8_palms_keep_cpu(self):
        broker = Mock(failures={'faces': 'bad network'}, names={'faces', 'objects'})
        factory = Mock(return_value=Mock())
        with self.assertLogs(hailo.LOG, 'WARNING'):
            faces = hailo.detector('faces', factory, Path('cpu.onnx'), broker)
        self.assertEqual(faces.backend, 'cpu')
        palm = hailo.detector('palms', factory, Path('palm.onnx'), broker)
        self.assertIs(palm, factory.return_value)

    def test_settings_default_auto_and_force_cpu(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'vision.json'
            self.assertEqual(accelerator_setting(path), 'auto')
            path.write_text('{"vision":{}}')
            self.assertEqual(accelerator_setting(path), 'auto')
            for value in ('auto', 'hailo', 'cpu'):
                path.write_text(json.dumps({'vision': {'accelerator': value}}))
                self.assertEqual(accelerator_setting(path), value)
            path.write_text('{"vision":{"accelerator":"gpu"}}')
            with self.assertLogs('orion_vision.service', 'WARNING'):
                self.assertEqual(accelerator_setting(path), 'cpu')

    def test_device_architecture_never_uses_board_name(self):
        output = 'Board Name: Hailo-8\nDevice Architecture: HAILO8L\n'
        with patch.object(models.subprocess, 'run', side_effect=[
                subprocess.CompletedProcess([], 0, '0001:01:00.0 1200: 1e60:2864'),
                subprocess.CompletedProcess([], 0, output)]):
            self.assertEqual(models.device_architecture(), 'HAILO8L')
        with self.assertRaisesRegex(RuntimeError, 'Device Architecture'):
            models.device_kind('Board Name: Hailo-8')


class SchedulingTests(unittest.TestCase):
    def test_hailo_face_rate_status_and_cpu_palm_rate(self):
        for mode, palm_backend, expected in (('faces', 'cpu', 15), ('hands', 'cpu', 5), ('hands', 'hailo', 15)):
            with self.subTest(mode=mode, palms=palm_backend):
                faces, palms, objects, client = Mock(), Mock(), Mock(), Mock()
                faces.backend = objects.backend = 'hailo'; palms.backend = palm_backend
                faces.detect.return_value = []; palms.detect.return_value = []
                client.voice_active.return_value = False
                pipeline = FacePipeline(faces, intrinsics(), client, objects=objects, palms=palms)
                pipeline.reset(True, mode=mode)
                pipeline.start()
                try:
                    pipeline.submit(1, np.zeros((360, 640, 3), np.uint8))
                    eventually(lambda: pipeline.rate_hz == expected)
                    self.assertEqual(pipeline.status()['backends'], {'faces': 'hailo', 'palms': palm_backend, 'objects': 'hailo'})
                    faces.backend = 'cpu'
                    pipeline.submit(2, np.zeros((360, 640, 3), np.uint8))
                    eventually(lambda: pipeline.rate_hz == (10 if mode == 'faces' else expected))
                    palms.backend = 'cpu'
                    pipeline.submit(3, np.zeros((360, 640, 3), np.uint8))
                    eventually(lambda: pipeline.rate_hz == (10 if mode == 'faces' else 5))
                finally:
                    pipeline.close()

    def test_object_cadence_changes_immediately_after_cpu_fallback(self):
        objects, client = Mock(), Mock()
        client.camera_pose.return_value = CAMERA
        objects.backend = 'hailo'
        pipeline = FacePipeline(Mock(), intrinsics(), client, objects=objects, clock=lambda: 100.)
        pipeline.reset(True, mode='faces')
        pipeline.behaviours['object_glances'] = True
        objects.detect.return_value = []
        pipeline._background_objects(1, None, [], pipeline.generation)
        self.assertEqual(pipeline.next_object_pass, 105.)
        pipeline.next_object_pass = 0
        def fallback(_):
            objects.backend = 'cpu'
            return []
        objects.detect.side_effect = fallback
        pipeline._background_objects(2, None, [], pipeline.generation)
        self.assertEqual(pipeline.next_object_pass, 160.)


class DeviceOwnerTests(unittest.TestCase):
    def test_multiplexed_requests_use_the_same_child_process(self):
        from orion_vision.inference_process import InferenceProcess
        worker = InferenceProcess(MultiplexFixture, None, name='Hailo')
        try:
            face_pid, name = worker.detect(('faces', 'pixels'))
            self.assertEqual(name, 'faces')
            object_pid, name = worker.detect(('objects', 'new pixels'))
            self.assertEqual(name, 'objects')
            self.assertEqual(face_pid, object_pid)
            self.assertNotEqual(face_pid, os.getpid())
        finally:
            worker.close()
        self.assertFalse(worker.process.is_alive())

    def test_one_vdevice_three_networks_nms_settings_and_per_network_error(self):
        device = Mock()
        device.__enter__ = Mock(return_value=device)
        device.__exit__ = Mock()
        group = Mock()
        device.configure.return_value = [group]
        vdevice = Mock(return_value=device)
        face_hef, object_hef, palm_hef = Mock(), Mock(), Mock()
        for hef in (face_hef, object_hef):
            hef.get_input_vstream_infos.return_value = [SimpleNamespace(name='input', shape=(640, 640, 3))]
        object_hef.get_output_vstream_infos.return_value = [SimpleNamespace(name='nms')]
        palm_hef.get_input_vstream_infos.return_value = [SimpleNamespace(name='input', shape=(192, 192, 3))]
        face_infer, object_infer, palm_infer = Mock(), Mock(), Mock()
        for inference in (face_infer, object_infer, palm_infer):
            inference.__enter__ = Mock(return_value=inference)
            inference.__exit__ = Mock()
        face_infer.infer.side_effect = RuntimeError('face network failed')
        object_infer.infer.return_value = {'nms': [[np.empty((0, 5)) for _ in range(80)]]}
        palm_infer.infer.return_value = {'reg': np.zeros((1, 2016, 18)), 'score': np.zeros((1, 2016, 1))}
        platform = SimpleNamespace(HEF=Mock(side_effect=[face_hef, object_hef, palm_hef]), VDevice=vdevice,
            HailoSchedulingAlgorithm=SimpleNamespace(ROUND_ROBIN='scheduled'),
            HailoStreamInterface=SimpleNamespace(PCIe='pcie'), ConfigureParams=Mock(),
            InputVStreamParams=Mock(), OutputVStreamParams=Mock(),
            FormatType=SimpleNamespace(UINT8='uint8', FLOAT32='float32'),
            InferVStreams=Mock(side_effect=[face_infer, object_infer, palm_infer]))
        with patch.dict(sys.modules, {'hailo_platform': platform}):
            owner = hailo._HailoNetworks({'faces': Path('face.hef'), 'objects': Path('object.hef'), 'palms': Path('palm.hef')})
            try:
                self.assertEqual(vdevice.call_count, 1)
                self.assertEqual(device.configure.call_count, 3)
                self.assertEqual(vdevice.create_params.return_value.scheduling_algorithm, 'scheduled')
                object_infer.set_nms_score_threshold.assert_called_once_with(SCORES['hailo']['objects'])
                object_infer.set_nms_max_proposals_per_class.assert_called_once_with(hailo.MAX_OBJECTS)
                self.assertTrue(all(call.kwargs['queue_size'] == 1 for call in platform.InputVStreamParams.make.call_args_list))
                image = np.zeros((360, 640, 3), np.uint8)
                self.assertIn('error', owner.detect(('faces', image)))
                self.assertEqual(owner.detect(('objects', image)), {'result': []})
                self.assertEqual(owner.detect(('palms', image)), {'result': []})
                palm_input = palm_infer.infer.call_args.args[0]['input']
                self.assertEqual(palm_input.shape, (1, 192, 192, 3))
                self.assertEqual(palm_input.dtype, np.uint8)
                self.assertIn('faces', owner.detect(('status', None)))
            finally:
                owner.close()
            device.__exit__.assert_called_once()
