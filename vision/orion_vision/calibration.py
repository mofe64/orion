"""Fisheye intrinsics for the exact 320x240 detection stream; no saved frames."""
import argparse
from dataclasses import dataclass
import json
from pathlib import Path
from threading import Condition
import time

import cv2
import numpy as np

SIZE = (320, 240)
DEFAULT_PATH = Path.home() / ".config/orion/camera-v2.json"
MAX_RMS = .5


def stream_geometry(metadata, geometry):
    """Numerical sensor/ISP geometry, copied from the configured camera."""
    try:
        value = dict(geometry)
        value['scaler_crop'] = list(metadata['ScalerCrop'])
        crop, sensor, flips, stride = (value[key] for key in
            ('scaler_crop', 'sensor_mode', 'flips', 'lores_stride'))
        size = list(sensor['output_size'])
        if (len(crop) != 4 or any(type(v) is not int for v in crop)
                or min(crop[:2]) < 0 or min(crop[2:]) <= 0 or len(size) != 2
                or any(type(v) is not int or v <= 0 for v in size)
                or type(sensor['bit_depth']) is not int or not 8 <= sensor['bit_depth'] <= 16
                or flips != {'hflip': False, 'vflip': False}
                or any(type(v) is not bool for v in flips.values())
                or type(stride) is not int or stride < SIZE[0] or stride % 2):
            raise ValueError()
        return {'scaler_crop': crop, 'sensor_mode': {'output_size': size,
            'bit_depth': sensor['bit_depth']}, 'flips': dict(flips), 'lores_stride': stride}
    except (KeyError, TypeError, ValueError):
        raise ValueError("Missing or unsupported calibration stream geometry (flips must be false)") from None


def validate_lores(buffer, stride=None):
    if not isinstance(buffer, np.ndarray) or buffer.dtype != np.uint8:
        raise ValueError("Lores buffer must contain uint8 pixels")
    if buffer.ndim == 3:
        if buffer.shape != (SIZE[1], SIZE[0], 3):
            raise ValueError("Replay lores must be exactly 320x240 RGB")
    elif (buffer.ndim != 2 or buffer.shape[0] != SIZE[1] * 3 // 2
          or buffer.shape[1] < SIZE[0] or buffer.shape[1] % 2
          or (stride is not None and buffer.shape[1] != stride)):
        raise ValueError("Lores YUV geometry must be 320x240 with its configured stride")


@dataclass
class Intrinsics:
    matrix: np.ndarray
    distortion: np.ndarray
    rms: float
    size: tuple = SIZE
    geometry: dict | None = None

    def __post_init__(self):
        self.matrix = np.asarray(self.matrix, dtype=np.float64).reshape(3, 3)
        self.distortion = np.asarray(self.distortion, dtype=np.float64).reshape(4, 1)
        if (tuple(self.size) != SIZE or not np.isfinite(self.matrix).all()
                or not np.isfinite(self.distortion).all() or not np.isfinite(self.rms)
                or not 0 <= self.rms <= MAX_RMS or min(self.matrix[0, 0], self.matrix[1, 1]) <= 0
                or not np.allclose(self.matrix[2], [0, 0, 1])):
            raise ValueError("Invalid 320x240 fisheye calibration or RMS above 0.5 px")
        if self.geometry is not None:
            self.geometry = stream_geometry({'ScalerCrop': self.geometry.get('scaler_crop')
                if isinstance(self.geometry, dict) else None}, self.geometry)

    @classmethod
    def load(cls, path):
        value = json.loads(Path(path).read_text())
        if value.get("model") != "opencv_fisheye" or value.get("schema_version") != 1:
            raise ValueError("Expected camera-v2 OpenCV fisheye calibration")
        if 'geometry' not in value:
            raise ValueError("Calibration has no stream geometry; refit camera-v2.json")
        geometry = stream_geometry({'ScalerCrop': value['geometry'].get('scaler_crop')
            if isinstance(value['geometry'], dict) else None}, value['geometry'])
        return cls(value["K"], value["D"], value["rms_px"], tuple(value["image_size"]), geometry)

    def save(self, path):
        # Only numerical calibration is saved, never checkerboard images.
        geometry = stream_geometry({'ScalerCrop': (self.geometry or {}).get('scaler_crop')}, self.geometry)
        path = Path(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = path.with_name(path.name + ".tmp")
        temporary.write_text(json.dumps({"schema_version": 1, "model": "opencv_fisheye",
            "image_size": list(self.size), "K": self.matrix.tolist(),
            "D": self.distortion.ravel().tolist(), "rms_px": float(self.rms),
            "geometry": geometry}, indent=2) + "\n")
        temporary.replace(path)

    def validate_capture(self, buffer, metadata, geometry):
        if stream_geometry(metadata, geometry) != self.geometry:
            raise ValueError("Capture geometry differs from camera-v2 calibration")
        validate_lores(buffer, self.geometry['lores_stride'])

    def rays(self, pixels):
        points = np.asarray(pixels, dtype=np.float64).reshape(-1, 1, 2)
        if not np.isfinite(points).all():
            raise ValueError("Pixels must be finite")
        xy = cv2.fisheye.undistortPoints(points, self.matrix, self.distortion).reshape(-1, 2)
        rays = np.column_stack((xy, np.ones(len(xy))))
        return rays / np.linalg.norm(rays, axis=1)[:, None]


def lores_bgr(buffer):
    validate_lores(buffer)
    # PiSP pads the width to a stride (384 on the commissioned lamp).
    # Keep the packed I420 planes intact until conversion; crop afterwards.
    if buffer.ndim == 3:
        return cv2.cvtColor(buffer, cv2.COLOR_RGB2BGR)  # synthetic replay
    return cv2.cvtColor(buffer, cv2.COLOR_YUV2BGR_I420)[:SIZE[1], :SIZE[0]]


def checkerboard_points(columns, rows, square_m):
    points = np.zeros((1, columns * rows, 3), dtype=np.float64)
    points[0, :, :2] = np.mgrid[:columns, :rows].T.reshape(-1, 2) * square_m
    return points


def fit(corners, columns, rows, square_m, geometry=None):
    if len(corners) < 12:
        raise ValueError("At least 12 distinct checkerboard views are required")
    objects = [checkerboard_points(columns, rows, square_m) for _ in corners]
    rms, matrix, distortion, _, _ = cv2.fisheye.calibrate(objects,
        [np.asarray(p, dtype=np.float64).reshape(1, -1, 2) for p in corners], SIZE,
        np.eye(3), np.zeros((4, 1)), flags=cv2.fisheye.CALIB_RECOMPUTE_EXTRINSIC
        | cv2.fisheye.CALIB_CHECK_COND | cv2.fisheye.CALIB_FIX_SKEW,
        criteria=(cv2.TERM_CRITERIA_EPS | cv2.TERM_CRITERIA_MAX_ITER, 100, 1e-7))
    return Intrinsics(matrix, distortion, rms, geometry=geometry)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--columns", type=int, default=9, help="Inner checkerboard corners")
    parser.add_argument("--rows", type=int, default=6)
    parser.add_argument("--square-mm", type=float, default=25)
    parser.add_argument("--views", type=int, default=24)
    parser.add_argument("--seconds", type=float, default=90)
    parser.add_argument("--output", type=Path, default=DEFAULT_PATH)
    args = parser.parse_args()
    if args.columns < 3 or args.rows < 3 or args.square_mm <= 0 or args.views < 12 or args.seconds <= 0:
        parser.error("Need a valid board, at least 12 views, and positive capture time")
    camera = json.loads(args.profile.read_text()).get("camera", {})
    if camera.get("enabled") is not True:
        parser.error("Profile has no enabled camera")
    if camera.get('hflip') or camera.get('vflip'):
        parser.error("Fisheye rays require a profile with both flips false")
    from .sources import Picamera2Source
    from .service import camera_enabled
    if not camera_enabled(Path.home() / ".config/orion/camera.json"):
        parser.error("Camera is disabled")
    cv2.setNumThreads(1)
    source = Picamera2Source(camera["index"], camera["hflip"], camera["vflip"])
    changed, latest = Condition(), [None]
    def publish(capture):
        with changed:
            latest[0] = capture
            changed.notify()
    corners, geometry = [], None
    deadline = time.monotonic() + args.seconds
    print("Move and tilt the board across the view, including the edges. Images remain in memory.", flush=True)
    try:
        source.start(publish, lambda: False, lambda: True)
        while len(corners) < args.views and time.monotonic() < deadline:
            if not camera_enabled(Path.home() / ".config/orion/camera.json"):
                raise RuntimeError("Camera was disabled during calibration")
            with changed:
                changed.wait_for(lambda: latest[0] is not None, timeout=.5)
                capture, latest[0] = latest[0], None
            if capture is None or capture.lores is None:
                continue
            measured = stream_geometry(capture.metadata, capture.geometry)
            if geometry is None:
                geometry = measured
            elif measured != geometry:
                raise ValueError("Camera geometry changed during calibration; retry with a fixed stream")
            buffer = capture.lores
            validate_lores(buffer, geometry['lores_stride'])
            gray = cv2.cvtColor(lores_bgr(buffer), cv2.COLOR_BGR2GRAY)
            found, points = cv2.findChessboardCorners(gray, (args.columns, args.rows))
            if found:
                points = cv2.cornerSubPix(gray, points, (5, 5), (-1, -1),
                    (cv2.TERM_CRITERIA_EPS | cv2.TERM_CRITERIA_MAX_ITER, 30, .01))
                if all(np.linalg.norm(points - old) / np.sqrt(len(points)) > 8 for old in corners):
                    corners.append(points)
                    print(f"Accepted view {len(corners)}/{args.views}", flush=True)
            time.sleep(.2)
    finally:
        source.close()
    intrinsics = fit(corners, args.columns, args.rows, args.square_mm / 1000, geometry)
    intrinsics.save(args.output)
    print(f"Saved {args.output}; RMS {intrinsics.rms:.3f} px")
