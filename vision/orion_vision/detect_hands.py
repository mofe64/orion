"""Pinned OpenCV Zoo MediaPipe palm detector; single CPU thread, bounded output.

Decoder adapted from opencv/opencv_zoo at 47534e27c9851bb1128ccc0102f1145e27f23f98,
models/palm_detection_mediapipe/mp_palmdet.py (Apache-2.0; see PALM-LICENSE).
Orion generates the equivalent SSD anchors and corrects NMS input to x/y/w/h.
"""
from dataclasses import dataclass
import hashlib
from pathlib import Path
import cv2
import numpy as np
from .intrinsics import SIZE, lores_bgr

MODEL_PATH = Path('/usr/local/share/orion/vision/palm_detection_mediapipe_2023feb.onnx')
MODEL_SHA256 = '78ff51c38496b7fc8b8ebdb6cc8c1abb02fa6c38427c6848254cdaba57fcce7c'
INPUT_SIZE = 192


@dataclass
class Palm:
    box: list
    confidence: float

    @property
    def centre(self):
        x, y, width, height = self.box
        return [x + width / 2, y + height / 2]


def anchors():
    return np.asarray([[(x + .5) / grid, (y + .5) / grid]
        for grid, copies in ((24, 2), (12, 6))
        for y in range(grid) for x in range(grid) for _ in range(copies)], np.float32)


def preprocess(buffer):
    image = lores_bgr(buffer)
    scale = INPUT_SIZE / max(SIZE)
    width, height = round(SIZE[0] * scale), round(SIZE[1] * scale)
    left, top = (INPUT_SIZE - width) // 2, (INPUT_SIZE - height) // 2
    padded = np.zeros((INPUT_SIZE, INPUT_SIZE, 3), np.uint8)
    padded[top:top + height, left:left + width] = cv2.resize(image, (width, height))
    blob = cv2.cvtColor(padded, cv2.COLOR_BGR2RGB).astype(np.float32)[None] / 255.
    return blob, (scale, left, top)


def decode(outputs, geometry):
    boxes, scores = (np.asarray(value) for value in outputs)
    if boxes.shape != (1, 2016, 18) or scores.shape != (1, 2016, 1):
        raise ValueError('Unexpected palm model outputs')
    score = 1 / (1 + np.exp(-np.clip(scores[0, :, 0].astype(float), -100, 100)))
    finite = np.isfinite(boxes[0]).all(axis=1) & np.isfinite(score)
    selected = np.flatnonzero(finite & (score >= .6))
    selected = selected[np.argsort(-score[selected])[:128]]
    if not len(selected):
        return []
    scale, left, top = geometry
    delta = boxes[0, selected, :4]
    centre = delta[:, :2] + anchors()[selected] * INPUT_SIZE
    xy = (centre - delta[:, 2:] / 2 - [left, top]) / scale
    wh = delta[:, 2:] / scale
    right_bottom = np.minimum(xy + wh, SIZE)
    xy = np.maximum(xy, 0)
    wh = right_bottom - xy
    valid = (wh > 1).all(axis=1)
    rectangles = np.column_stack((xy[valid], wh[valid])).astype(float)
    probabilities = score[selected][valid]
    kept = cv2.dnn.NMSBoxes(rectangles.tolist(), probabilities.tolist(), .6, .3, top_k=128)
    return [Palm(rectangles[i].tolist(), float(probabilities[i])) for i in np.asarray(kept).reshape(-1)[:8]]


class PalmDetector:
    def __init__(self, path=MODEL_PATH):
        if hashlib.sha256(Path(path).read_bytes()).hexdigest() != MODEL_SHA256:
            raise ValueError('Palm model SHA-256 mismatch')
        cv2.setNumThreads(1)
        self.model = cv2.dnn.readNet(str(path))
        self.model.setPreferableBackend(cv2.dnn.DNN_BACKEND_OPENCV)
        self.model.setPreferableTarget(cv2.dnn.DNN_TARGET_CPU)

    def detect(self, buffer):
        blob, geometry = preprocess(buffer)
        self.model.setInput(blob)
        return decode(self.model.forward(self.model.getUnconnectedOutLayersNames()), geometry)
