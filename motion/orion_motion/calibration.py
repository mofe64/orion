"""Read Orion's accepted STS3215 calibration as position-limit authority."""

from __future__ import annotations

import json
import math
from dataclasses import dataclass
from pathlib import Path
from typing import Any


ENCODER_RESOLUTION = 4096
CANONICAL_JOINT_NAMES = (
    "base_yaw_joint",
    "shoulder_pitch_joint",
    "elbow_pitch_joint",
    "head_roll_joint",
    "head_pitch_joint",
)


class CalibrationError(ValueError):
    """Raised when a calibration cannot safely define Orion joint ranges."""


@dataclass(frozen=True)
class CalibratedJointRange:
    name: str
    lower_rad: float
    upper_rad: float


def load_calibrated_joint_ranges(path: Path) -> tuple[CalibratedJointRange, ...]:
    """Return safe joint-coordinate ranges from one accepted calibration."""

    try:
        root: Any = json.loads(path.expanduser().read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise CalibrationError(f"Could not read calibration '{path}': {exc}") from exc
    if not isinstance(root, dict):
        raise CalibrationError("Calibration must be a JSON object.")
    if (
        root.get("schema_version") != 1
        or root.get("robot") != "orion"
        or root.get("servo_model") != "sts3215"
        or root.get("encoder_resolution") != ENCODER_RESOLUTION
        or root.get("writes_servo_eeprom") is not False
    ):
        raise CalibrationError(
            "Calibration must be Orion STS3215 schema 1 with 4096-count software-only provenance."
        )
    joints = root.get("joints")
    if not isinstance(joints, dict) or set(joints) != set(CANONICAL_JOINT_NAMES):
        raise CalibrationError("Calibration joints do not match Orion.")

    radians_per_step = math.tau / ENCODER_RESOLUTION
    ranges: list[CalibratedJointRange] = []
    for name in CANONICAL_JOINT_NAMES:
        joint = joints[name]
        if not isinstance(joint, dict):
            raise CalibrationError(f"Calibration entry for {name} must be an object.")
        minimum = joint.get("safe_min_delta_raw")
        maximum = joint.get("safe_max_delta_raw")
        direction = joint.get("encoder_direction")
        if (
            type(minimum) is not int
            or type(maximum) is not int
            or direction not in (-1, 1)
            or not -2048 < minimum < 0 < maximum < 2048
        ):
            raise CalibrationError(f"Calibration entry for {name} has an invalid safe range.")
        first = minimum * radians_per_step / direction
        second = maximum * radians_per_step / direction
        ranges.append(
            CalibratedJointRange(name, min(first, second), max(first, second))
        )
    return tuple(ranges)
