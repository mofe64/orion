"""Built-in Camera Module 3 Wide optics and configured capture geometry."""
from dataclasses import dataclass
import math

import cv2
import numpy as np

SIZE = (640, 360)
SENSOR_SIZE = (4608, 2592)
SENSOR_OUTPUT_SIZE = (2304, 1296)  # Full-view 2x2 binning, not cropped 1536x864.
SCALER_CROP = (0, 0, *SENSOR_SIZE)
FOCAL_LENGTH_MM = 2.75
PIXEL_PITCH_MM = .0014
HORIZONTAL_FOV_DEGREES = 102.
# Raspberry Pi Camera Module 3 product brief, Wide variants:
# https://datasheets.raspberrypi.com/camera/camera-module-3-product-brief.pdf


def camera_geometry():
    """Settings requested by the service; the source records negotiated stride."""
    return {'scaler_crop': list(SCALER_CROP),
        'sensor_mode': {'output_size': list(SENSOR_OUTPUT_SIZE), 'bit_depth': 10},
        'flips': {'hflip': False, 'vflip': False}, 'lores_stride': SIZE[0]}


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
        raise ValueError("Missing or unsupported capture geometry (flips must be false)") from None


def validate_lores(buffer, stride=None):
    if not isinstance(buffer, np.ndarray) or buffer.dtype != np.uint8:
        raise ValueError("Lores buffer must contain uint8 pixels")
    if buffer.ndim == 3:
        if buffer.shape != (SIZE[1], SIZE[0], 3):
            raise ValueError(f"Replay lores must be exactly {SIZE[0]}x{SIZE[1]} RGB")
    elif (buffer.ndim != 2 or buffer.shape[0] != SIZE[1] * 3 // 2
          or buffer.shape[1] < SIZE[0] or buffer.shape[1] % 2
          or (stride is not None and buffer.shape[1] != stride)):
        raise ValueError(f"Lores YUV geometry must be {SIZE[0]}x{SIZE[1]} with its configured stride")


@dataclass
class Intrinsics:
    matrix: np.ndarray
    distortion: np.ndarray
    geometry: dict | None = None

    def __post_init__(self):
        self.matrix = np.asarray(self.matrix, dtype=np.float64).reshape(3, 3)
        self.distortion = np.asarray(self.distortion, dtype=np.float64).reshape(4, 1)
        if (not np.isfinite(self.matrix).all() or not np.isfinite(self.distortion).all()
                or min(self.matrix[0, 0], self.matrix[1, 1]) <= 0
                or not np.allclose(self.matrix[2], [0, 0, 1])):
            raise ValueError("Invalid fisheye intrinsics")
        if self.geometry is not None:
            self.geometry = stream_geometry({'ScalerCrop': self.geometry.get('scaler_crop')
                if isinstance(self.geometry, dict) else None}, self.geometry)

    @classmethod
    def camera_module_3_wide(cls):
        """Nominal OpenCV fisheye model derived from the published wide lens.

        The near-axis focal length is f_mm / pixel_pitch_mm. Choose k1 so
        theta_d = theta * (1 + k1 * theta**2) reaches the sensor's horizontal
        edge at half the published FOV. Higher coefficients are zero; these
        are nominal specifications, not measurements of an individual lens.
        """
        geometry = camera_geometry()
        sensor_focal = FOCAL_LENGTH_MM / PIXEL_PITCH_MM
        half_fov = math.radians(HORIZONTAL_FOV_DEGREES / 2)
        k1 = (SENSOR_SIZE[0] / (2 * sensor_focal * half_fov) - 1) / half_fov**2

        # ScalerCrop uses full-sensor coordinates. First map into the selected
        # binned mode, then crop and independently scale to the lores output.
        mode_scale = np.asarray(geometry['sensor_mode']['output_size']) / SENSOR_SIZE
        crop = np.asarray(geometry['scaler_crop'])
        output_scale = np.asarray(SIZE) / (crop[2:] * mode_scale)
        focal = sensor_focal * mode_scale * output_scale
        centre = (np.asarray(SENSOR_SIZE) / 2 - crop[:2]) * mode_scale * output_scale
        return cls([[focal[0], 0, centre[0]], [0, focal[1], centre[1]], [0, 0, 1]],
            [k1, 0, 0, 0], geometry)

    def validate_capture(self, buffer, metadata, geometry):
        if stream_geometry(metadata, geometry) != self.geometry:
            raise ValueError("Capture geometry differs from the built-in camera configuration")
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
    # PiSP may pad the SIZE[0]-pixel width to the negotiated lores stride.
    # Keep the packed I420 planes intact until conversion; crop afterwards.
    if buffer.ndim == 3:
        return cv2.cvtColor(buffer, cv2.COLOR_RGB2BGR)  # synthetic replay
    return cv2.cvtColor(buffer, cv2.COLOR_YUV2BGR_I420)[:SIZE[1], :SIZE[0]]
