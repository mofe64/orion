"""One Hailo owner, CPU image geometry and the existing detector result types."""
from contextlib import ExitStack
import logging

import numpy as np

from .detect_faces import Face
from .detect_objects import CATEGORIES, Object
from .detect_hands import INPUT_SIZE as PALM_SIZE, preprocess as palm_preprocess, decode as palm_decode
from .detector_policy import SCORES
from .hailo_models import MODEL_DIRECTORY, checked_model_paths, device_architecture
from .inference_process import InferenceProcess
from .intrinsics import SIZE, lores_bgr
from .postprocessing import nms

LOG = logging.getLogger(__name__)
INPUT_SIZE = 640
FACE_NMS_IOU = .4
OBJECT_NMS_IOU = .45
MAX_FACES = 16
MAX_OBJECTS = 16


def preprocess(pixels, fill):
    """RGB UINT8; normalization is already in both pinned HEF graphs."""
    image = lores_bgr(pixels)[..., ::-1]
    scale = INPUT_SIZE / max(SIZE)
    width, height = round(SIZE[0] * scale), round(SIZE[1] * scale)
    # Current lores width equals the network width. Keep resize on the CPU if
    # capture dimensions change; interpolate pixel centres, not image corners.
    if image.shape[:2] != (height, width):
        x = np.clip((np.arange(width) + .5) / scale - .5, 0, SIZE[0] - 1)
        y = np.clip((np.arange(height) + .5) / scale - .5, 0, SIZE[1] - 1)
        x0, y0 = x.astype(int), y.astype(int)
        x1, y1 = np.minimum(x0 + 1, SIZE[0] - 1), np.minimum(y0 + 1, SIZE[1] - 1)
        wx, wy = (x - x0)[None, :, None], (y - y0)[:, None, None]
        image = np.rint((1 - wy) * ((1 - wx) * image[y0[:, None], x0] + wx * image[y0[:, None], x1])
                        + wy * ((1 - wx) * image[y1[:, None], x0] + wx * image[y1[:, None], x1])).astype(np.uint8)
    left, top = (INPUT_SIZE - width) // 2, (INPUT_SIZE - height) // 2
    padded = np.full((INPUT_SIZE, INPUT_SIZE, 3), fill, np.uint8)
    padded[top:top + height, left:left + width] = image
    return padded[None], (scale, left, top)


def frame_boxes(boxes, geometry):
    scale, left, top = geometry
    boxes = (np.asarray(boxes, dtype=float) - [left, top, left, top]) / scale
    boxes[:, [0, 2]] = np.clip(boxes[:, [0, 2]], 0, SIZE[0])
    boxes[:, [1, 3]] = np.clip(boxes[:, [1, 3]], 0, SIZE[1])
    boxes[:, 2:] -= boxes[:, :2]
    return boxes


def decode_faces(outputs, geometry):
    """SCRFD 2.5G: three strides, two anchors, four distances, five points.

    Output names vary across HEFs. Spatial shape and channel count identify
    each tensor without depending on dictionary order or compiler node names.
    Sigmoid scores and input normalization are part of the published network.
    """
    tensors = {}
    for value in outputs.values():
        value = np.asarray(value, dtype=float)
        if value.ndim != 4 or value.shape[0] != 1:
            raise ValueError('Unexpected SCRFD tensor shape')
        key = value.shape[1:]
        if key in tensors:
            raise ValueError('Duplicate SCRFD tensor shape')
        tensors[key] = value[0]
    expected = {(INPUT_SIZE // stride, INPUT_SIZE // stride, channels)
                for stride in (8, 16, 32) for channels in (8, 2, 20)}
    if set(tensors) != expected:
        raise ValueError('SCRFD requires boxes, scores and five landmarks at every stride')
    boxes, scores, landmarks = [], [], []
    for stride in (8, 16, 32):
        grid = INPUT_SIZE // stride
        anchors = np.stack(np.mgrid[:grid, :grid][::-1], axis=-1).reshape(-1, 2)
        anchors = np.repeat(anchors * stride, 2, axis=0)
        distance = tensors[grid, grid, 8].reshape(-1, 4) * stride
        points = tensors[grid, grid, 20].reshape(-1, 5, 2) * stride + anchors[:, None]
        boxes.append(np.column_stack((anchors - distance[:, :2], anchors + distance[:, 2:])))
        scores.append(tensors[grid, grid, 2].reshape(-1))
        landmarks.append(points)
    rectangles = frame_boxes(np.concatenate(boxes), geometry)
    probabilities = np.concatenate(scores)
    scale, left, top = geometry
    points = (np.concatenate(landmarks) - [left, top]) / scale
    valid = (np.isfinite(rectangles).all(axis=1) & np.isfinite(points).all(axis=(1, 2))
             & np.isfinite(probabilities) & (probabilities >= SCORES['hailo']['faces'])
             & (probabilities <= 1) & (rectangles[:, 2:] > 1).all(axis=1))
    rectangles, probabilities, points = rectangles[valid], probabilities[valid], points[valid]
    kept = nms(rectangles, probabilities, FACE_NMS_IOU, MAX_FACES)
    kept.sort(key=lambda index: -rectangles[index, 2:].prod())
    return [Face(rectangles[i].tolist(), points[i].tolist(), float(probabilities[i])) for i in kept]


def decode_objects(classes, geometry, allowed_classes=None):
    """HailoRT NMS: 80 class arrays of normalized ymin,xmin,ymax,xmax,score.

    HailoRT performs the HEF's YOLO box decoding and NMS on the CPU. Orion
    restores capture coordinates, filters scores/classes and bounds results.
    """
    if len(classes) != len(CATEGORIES):
        raise ValueError('Expected 80 COCO classes from HailoRT NMS')
    detections = []
    for category, values in zip(CATEGORIES, classes):
        rows = np.asarray(values, dtype=float)
        if rows.size == 0:
            continue
        if rows.ndim != 2 or rows.shape[1] != 5:
            raise ValueError('Unexpected HailoRT NMS box shape')
        if allowed_classes is not None and category not in allowed_classes:
            continue
        valid = (np.isfinite(rows).all(axis=1) & (rows[:, 4] >= SCORES['hailo']['objects'])
                 & (rows[:, 4] <= 1))
        rows = rows[valid]
        boxes = frame_boxes(rows[:, [1, 0, 3, 2]] * INPUT_SIZE, geometry)
        for box, score in zip(boxes, rows[:, 4]):
            if (box[2:] > 1).all():
                detections.append(Object(category, box.tolist(), float(score)))
    return sorted(detections, key=lambda item: -item.confidence)[:MAX_OBJECTS]


def decode_palms(outputs, geometry):
    """Join the supplied HEF's stride-8/16 heads in MediaPipe anchor order.

    The output transforms can flatten spatial axes. Element counts identify
    each head without depending on compiler node names or dictionary order.
    The CPU and Hailo paths then share sigmoid, anchors, boxes and NumPy NMS.
    """
    tensors = {}
    for value in outputs.values():
        tensor = np.asarray(value)
        if tensor.ndim < 2 or tensor.shape[0] != 1 or tensor.size in tensors:
            raise ValueError('Unexpected palm HEF tensor shape')
        tensors[tensor.size] = tensor
    if set(tensors) == {2016, 2016 * 18}:
        boxes, scores = tensors[2016 * 18].reshape(1, 2016, 18), tensors[2016].reshape(1, 2016, 1)
    elif set(tensors) == {1152, 864, 1152 * 18, 864 * 18}:
        boxes = np.concatenate([tensors[n * 18].reshape(1, n, 18) for n in (1152, 864)], axis=1)
        scores = np.concatenate([tensors[n].reshape(1, n, 1) for n in (1152, 864)], axis=1)
    else:
        raise ValueError('Expected raw MediaPipe palm regression and score heads')
    return palm_decode((boxes, scores), geometry, threshold=SCORES['hailo']['palms'])


class _HailoNetworks:
    """Created only inside the child. One VDevice owns every network group."""
    def __init__(self, paths):
        from hailo_platform import (HEF, VDevice, HailoSchedulingAlgorithm, HailoStreamInterface,
                                    ConfigureParams, InputVStreamParams, OutputVStreamParams,
                                    FormatType, InferVStreams)
        self.resources = ExitStack()
        self.networks, self.failures = {}, {}
        try:
            params = VDevice.create_params()
            params.scheduling_algorithm = HailoSchedulingAlgorithm.ROUND_ROBIN
            device = self.resources.enter_context(VDevice(params))
            for name, path in paths.items():
                resources = ExitStack()
                try:
                    hef = HEF(str(path))
                    group, = device.configure(hef, ConfigureParams.create_from_hef(hef, HailoStreamInterface.PCIe))
                    input_info, = hef.get_input_vstream_infos()
                    size = PALM_SIZE if name == 'palms' else INPUT_SIZE
                    if tuple(input_info.shape) != (size, size, 3):
                        raise ValueError('Unexpected Hailo input shape')
                    inputs = InputVStreamParams.make(group, format_type=FormatType.UINT8, timeout_ms=2000, queue_size=1)
                    outputs = OutputVStreamParams.make(group, format_type=FormatType.FLOAT32, timeout_ms=2000, queue_size=1)
                    inference = resources.enter_context(InferVStreams(group, inputs, outputs))
                    if name == 'objects':
                        output_info, = hef.get_output_vstream_infos()
                        inference.set_nms_score_threshold(SCORES['hailo']['objects'])
                        inference.set_nms_iou_threshold(OBJECT_NMS_IOU)
                        inference.set_nms_max_proposals_per_class(MAX_OBJECTS)
                    self.networks[name] = (inference, input_info.name)
                    self.resources.callback(resources.close)
                except Exception as error:
                    resources.close()
                    self.failures[name] = f'{type(error).__name__}: {error}'
        except BaseException:
            self.resources.close()
            raise

    def detect(self, request):
        name, pixels = request
        if name == 'status':
            return self.failures
        if name in self.failures:
            return {'error': self.failures[name]}
        try:
            inference, input_name = self.networks[name]
            if name == 'palms':
                # The HEF contains /255 normalization; supply UINT8 RGB.
                image, geometry = palm_preprocess(pixels, normalise=False)
            else:
                image, geometry = preprocess(pixels, 114 if name == 'objects' else 0)
            outputs = inference.infer({input_name: image})
            if name == 'faces':
                result = decode_faces(outputs, geometry)
            elif name == 'palms':
                result = decode_palms(outputs, geometry)
            else:
                classes, = outputs.values()
                result = decode_objects(classes[0], geometry)
            return {'result': result}
        except Exception as error:
            self.failures[name] = f'{type(error).__name__}: {error}'
            return {'error': self.failures[name]}

    def close(self):
        self.resources.close()


def select_accelerator(accelerator='auto', directory=MODEL_DIRECTORY):
    if accelerator not in ('auto', 'hailo', 'cpu'):
        raise ValueError('vision.accelerator must be auto, hailo or cpu')
    if accelerator == 'cpu':
        return None
    try:
        architecture = device_architecture()
        if architecture is None:
            if accelerator == 'hailo':
                raise RuntimeError('No Hailo device')
            return None
        paths = checked_model_paths(architecture, directory)
        process = InferenceProcess(_HailoNetworks, paths, name='Hailo')
        process.names = set(paths)
        try:
            process.failures = process.detect(('status', None))
        except BaseException:
            process.close()
            raise
        return process
    except Exception as error:
        LOG.warning('Hailo unavailable; using CPU detectors: %s', error)
        return None


class HailoDetector:
    """A detector view of the shared child, with a one-way CPU fallback."""
    backend = 'hailo'

    def __init__(self, name, process, cpu_factory):
        self.name, self.process, self.cpu_factory = name, process, cpu_factory
        self.cpu = None
        self.closed = False
        if name in process.failures:
            self._fallback(process.failures[name])

    def _fallback(self, error):
        if self.closed:
            raise RuntimeError(f'{self.name} detector closed')
        self.backend = 'cpu'
        LOG.warning('%s: Hailo failed; using CPU: %s', self.name, error)
        self.cpu = self.cpu_factory()

    @property
    def available(self):
        return not self.closed and getattr(self.cpu, 'available', True)

    def detect(self, pixels):
        if self.closed:
            raise RuntimeError(f'{self.name} detector closed')
        if self.backend == 'hailo':
            try:
                response = self.process.detect((self.name, pixels))
                if 'error' in response:
                    raise RuntimeError(response['error'])
                return response['result']
            except Exception as error:
                self._fallback(error)
        if self.cpu is None:
            raise RuntimeError(f'{self.name}: CPU fallback unavailable')
        return self.cpu.detect(pixels)

    def close(self):
        self.closed = True
        if self.cpu is not None and hasattr(self.cpu, 'close'):
            self.cpu.close()


def detector(name, factory, path, process):
    # HAILO8 has no shipped palm HEF; select each detector independently.
    if process is not None and name in process.names:
        return HailoDetector(name, process, lambda: factory(path))
    return factory(path)
