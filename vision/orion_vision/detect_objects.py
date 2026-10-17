"""Pinned YOLO11n NCNN export: one CPU thread, 320-square letterbox, bounded NMS.

The model and its Ultralytics export are AGPL-3.0; Orion is GPL-3.0.
No weights, frames, or inference outputs are written by this module.
"""
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path

import cv2
import numpy as np

from .intrinsics import SIZE, lores_bgr
from .inference_process import InferenceProcess
from .detector_policy import SCORES

MODEL_PATH = Path('/usr/local/share/orion/vision/yolo11n_ncnn_model')
MODEL_HASHES = {
    'model.ncnn.param': '1f2df69cee32d80ed897bf67da1d945b29a70b6bf068594e720c990b445a77d9',
    'model.ncnn.bin': 'e03a54d4028ca4c52ad5676ceebe4f5e8df7accf9f00187ec00f2aa4c91e8225',
}
CATEGORIES = tuple(json.loads(Path(__file__).with_name('coco.json').read_text()))
SCORE_THRESHOLD, NMS_THRESHOLD = SCORES['cpu']['objects'], .45


@dataclass
class Object:
    category: str
    box: list  # x, y, width, height in the original lores image
    confidence: float

    @property
    def centre(self):
        x, y, width, height = self.box
        return [x + width / 2, y + height / 2]


def decode(rows):
    """The pinned export yields [4 xywh + 80 class scores, 2100 anchors]."""
    rows = np.asarray(rows)
    if rows.shape != (84, 2100):
        raise ValueError(f'Unexpected YOLO11n output shape: {rows.shape}')
    rows = rows.T
    classes = rows[:, 4:].argmax(axis=1)
    scores = rows[np.arange(len(rows)), classes + 4]
    valid = (np.isfinite(rows).all(axis=1) & (scores >= SCORE_THRESHOLD)
             & (scores <= 1) & (rows[:, 2:4] > 0).all(axis=1))
    indices = np.flatnonzero(valid)
    # Bound the quadratic suppression work even on adversarial model output.
    indices = indices[np.argsort(-scores[indices], kind='stable')[:300]]
    boxes, selected_classes, selected_scores = [], [], []
    scale = 320 / SIZE[0]
    top = (320 - round(SIZE[1] * scale)) // 2
    for index in indices:
        x, y, width, height = rows[index, :4]
        left = float(np.clip((x - width / 2) / scale, 0, SIZE[0]))
        right = float(np.clip((x + width / 2) / scale, 0, SIZE[0]))
        upper = float(np.clip((y - height / 2 - top) / scale, 0, SIZE[1]))
        lower = float(np.clip((y + height / 2 - top) / scale, 0, SIZE[1]))
        if right <= left or lower <= upper:
            continue
        boxes.append([left, upper, right - left, lower - upper])
        selected_classes.append(int(classes[index]))
        selected_scores.append(float(scores[index]))
    kept = []
    for category in sorted(set(selected_classes)):
        group = [i for i, value in enumerate(selected_classes) if value == category]
        chosen = cv2.dnn.NMSBoxes([boxes[i] for i in group],
            [selected_scores[i] for i in group], SCORE_THRESHOLD, NMS_THRESHOLD)
        kept.extend(group[int(i)] for i in np.asarray(chosen).reshape(-1))
    kept.sort(key=lambda i: -selected_scores[i])
    return [Object(CATEGORIES[selected_classes[i]], boxes[i], selected_scores[i]) for i in kept[:16]]


class _NativeYoloNCNN:
    def __init__(self, path=MODEL_PATH):
        import ncnn
        for name, expected in MODEL_HASHES.items():
            if hashlib.sha256((Path(path) / name).read_bytes()).hexdigest() != expected:
                raise ValueError(f'YOLO11n {name} SHA-256 mismatch')
        self.ncnn = ncnn
        cv2.setNumThreads(1)
        self.net = ncnn.Net()
        self.net.opt.num_threads = 1
        self.net.opt.use_vulkan_compute = False
        if (self.net.load_param(str(Path(path) / 'model.ncnn.param')) != 0
                or self.net.load_model(str(Path(path) / 'model.ncnn.bin')) != 0):
            raise ValueError('Could not load YOLO11n NCNN export')

    def detect(self, buffer):
        bgr = lores_bgr(buffer)
        height = round(SIZE[1] * 320 / SIZE[0])
        resized = cv2.resize(bgr, (320, height), interpolation=cv2.INTER_AREA)
        top = (320 - height) // 2
        square = cv2.copyMakeBorder(resized, top, 320 - height - top, 0, 0,
                                   cv2.BORDER_CONSTANT, value=(114, 114, 114))
        tensor = self.ncnn.Mat.from_pixels(square,
            self.ncnn.Mat.PixelType.PIXEL_BGR2RGB, 320, 320)
        tensor.substract_mean_normalize([], [1 / 255] * 3)
        with self.net.create_extractor() as extractor:
            if extractor.input('in0', tensor) != 0:
                raise ValueError('NCNN rejected YOLO input')
            status, output = extractor.extract('out0')
            if status != 0:
                raise ValueError('NCNN rejected YOLO output')
            return decode(np.asarray(output))


class YoloNCNN(InferenceProcess):
    """Isolate NCNN's GIL-holding extraction from capture and the watchdog."""
    def __init__(self, path=MODEL_PATH):
        super().__init__(_NativeYoloNCNN, path)
