"""A single explicit lookup, serviced only by the existing inference worker."""
from dataclasses import dataclass, field
from threading import Event

import numpy as np


class FindError(ValueError):
    def __init__(self, code, message):
        self.code = code
        super().__init__(message)


@dataclass
class FindJob:
    category: str
    session: str | None
    generation: int
    deadline: float
    done: Event = field(default_factory=Event)
    result: dict | None = None
    error: Exception | None = None
    running: bool = False


def locate(objects, intrinsics, camera, stamp):
    """Rigid transform supplied by runtime FK at capture, never model geometry.

    Preserve this small transform before slow inference outlives pose history.
    Rays use the same forward optical convention as FramePose::table_point.
    """
    origin = np.asarray(camera['position'], dtype=float)
    rotation = np.asarray(camera['rotation'], dtype=float)
    if (origin.shape != (3,) or rotation.shape != (3, 3)
            or not np.isfinite(origin).all() or not np.isfinite(rotation).all()
            or not np.allclose(rotation.T @ rotation, np.eye(3), atol=1e-6)
            or not np.isclose(np.linalg.det(rotation), 1, atol=1e-6)):
        raise ValueError('Runtime camera transform is invalid')
    rays = intrinsics.rays([obj.centre for obj in objects]) if objects else []
    candidates = []
    for obj, ray in zip(objects, rays):
        direction = rotation @ ray
        point = None
        if ray[2] > 0 and direction[2] < -1e-6:
            distance = -origin[2] / direction[2]
            if distance > 0:
                point = (origin + distance * direction).tolist()
                point[2] = 0.0  # Preserve the plane exactly despite round-off.
        candidates.append({'category': obj.category, 'box': obj.box,
            'confidence': obj.confidence, 'point': point,
            'azimuth': float(np.arctan2(-direction[0], -direction[1])),
            'elevation': float(np.arctan2(direction[2], np.hypot(*direction[:2]))),
            'capture_boottime_ns': str(stamp)})
    return candidates
