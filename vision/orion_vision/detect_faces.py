"""Pinned YuNet detector, limited to sixteen faces and one OpenCV thread."""
from dataclasses import dataclass
import hashlib
from pathlib import Path

import cv2
import numpy as np

from .intrinsics import SIZE, lores_bgr

MODEL_PATH = Path("/usr/local/share/orion/vision/face_detection_yunet_2023mar.onnx")
MODEL_SHA256 = "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4"


@dataclass
class Face:
    box: list
    landmarks: list
    confidence: float

    @property
    def centre(self):
        x, y, width, height = self.box
        return [x + width / 2, y + height / 2]

    @property
    def looking_at_orion(self):
        eyes = np.asarray(self.landmarks[:2])
        nose = np.asarray(self.landmarks[2])
        mouth = np.asarray(self.landmarks[3:])
        span = np.linalg.norm(eyes[1] - eyes[0])
        if span < 1:
            return False
        # Symmetry is a coarse frontal-face cue, not an eye-gaze estimator.
        axis = (eyes[1] - eyes[0]) / span
        return bool(abs(np.dot(nose - eyes.mean(axis=0), axis)) < .18 * span
                and abs(np.dot(mouth.mean(axis=0) - eyes.mean(axis=0), axis)) < .25 * span)


class YuNet:
    def __init__(self, path=MODEL_PATH):
        if hashlib.sha256(Path(path).read_bytes()).hexdigest() != MODEL_SHA256:
            raise ValueError("YuNet model SHA-256 mismatch")
        cv2.setNumThreads(1)
        self.model = cv2.FaceDetectorYN.create(str(path), "", SIZE, .9, .3, 5000,
            cv2.dnn.DNN_BACKEND_OPENCV, cv2.dnn.DNN_TARGET_CPU)

    def detect(self, buffer):
        _, rows = self.model.detect(lores_bgr(buffer))
        if rows is None:
            return []
        rows = sorted(rows, key=lambda row: float(row[2] * row[3]), reverse=True)[:16]
        return [Face(row[:4].astype(float).tolist(), row[4:14].reshape(5, 2).astype(float).tolist(),
            float(row[14])) for row in rows if np.isfinite(row).all() and min(row[2:4]) > 0]
