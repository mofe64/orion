"""Centre each servo's encoder frame on its calibrated zero.

The STS3215 positions absolutely within raw 0..4095 and never wraps. If a
joint's commandable range crosses raw 0/4095, a goal on the far side of that
boundary drives the servo almost a full turn the long way, into the opposite
end stop. A persistent homing offset (the Ofs register) shifts the servo's
reported and commanded positions so the calibrated zero reads raw 2047 and
every range sits well inside 0..4095.

The servo reports ``present = encoder - offset`` (mod 4096). Joint deltas do
not change, so poses stored in radians stay valid after centring.
"""

from __future__ import annotations

from collections.abc import Mapping
from copy import deepcopy
from dataclasses import dataclass
from datetime import UTC, datetime

from .calibration import (
    ENCODER_RESOLUTION,
    HALF_TURN_RAW,
    LELAMP_HOMING_TARGET_RAW,
    CalibrationError,
    crosses_encoder_boundary,
)

MAX_HOMING_OFFSET_RAW = 2047  # Ofs is 11-bit magnitude with a sign bit.


@dataclass(frozen=True)
class CentringStep:
    """The offset change for one joint."""

    joint_name: str
    servo_id: int
    neutral_raw: int
    current_offset_raw: int
    target_offset_raw: int
    lowest_raw: int
    highest_raw: int

    @property
    def crosses_boundary(self) -> bool:
        return self.lowest_raw < 0 or self.highest_raw >= ENCODER_RESOLUTION

    @property
    def changes_offset(self) -> bool:
        return self.current_offset_raw != self.target_offset_raw


def _joint_int(joint: Mapping[str, object], name: str, field: str) -> int:
    value = joint.get(field)
    if type(value) is not int:
        raise CalibrationError(f"{name}.{field} must be an integer.")
    return value


def plan_centring(document: Mapping[str, object]) -> tuple[CentringStep, ...]:
    """Compute the offset that puts every joint's calibrated zero at raw 2047."""

    if (
        document.get("schema_version") != 1
        or document.get("robot") != "orion"
        or document.get("servo_model") != "sts3215"
        or document.get("encoder_resolution") != ENCODER_RESOLUTION
    ):
        raise CalibrationError("Calibration is not an Orion STS3215 schema-version-1 file.")
    joints = document.get("joints")
    if not isinstance(joints, dict) or not joints:
        raise CalibrationError("Calibration joints must be a non-empty mapping.")

    steps = []
    for name, joint in joints.items():
        if not isinstance(joint, dict):
            raise CalibrationError(f"Calibration {name} must be a mapping.")
        servo_id = _joint_int(joint, name, "servo_id")
        neutral = _joint_int(joint, name, "neutral_raw")
        safe_min = _joint_int(joint, name, "safe_min_delta_raw")
        safe_max = _joint_int(joint, name, "safe_max_delta_raw")
        current = joint.get("servo_homing_offset_raw", 0)
        if type(current) is not int:
            raise CalibrationError(f"{name}.servo_homing_offset_raw must be an integer.")
        # Encoder position at calibrated zero, independent of any old offset.
        encoder_neutral = neutral + current
        target = (
            (encoder_neutral - LELAMP_HOMING_TARGET_RAW + HALF_TURN_RAW) % ENCODER_RESOLUTION
            - HALF_TURN_RAW
        )
        if abs(target) > MAX_HOMING_OFFSET_RAW:
            raise CalibrationError(f"{name} needs offset {target}, outside the Ofs register range.")
        steps.append(
            CentringStep(
                joint_name=name,
                servo_id=servo_id,
                neutral_raw=neutral,
                current_offset_raw=current,
                target_offset_raw=target,
                lowest_raw=neutral + safe_min,
                highest_raw=neutral + safe_max,
            )
        )
    return tuple(sorted(steps, key=lambda step: step.servo_id))


def shifted_raw(raw: int, old_offset: int, new_offset: int) -> int:
    """Where a raw reading under ``old_offset`` reads under ``new_offset``."""

    return (raw + old_offset - new_offset) % ENCODER_RESOLUTION


def centred_document(
    document: Mapping[str, object],
    steps: tuple[CentringStep, ...],
    *,
    centred_at: datetime | None = None,
) -> dict[str, object]:
    """Rewrite the calibration into the centred raw frame.

    Joint deltas, safe ranges and LeRobot range equivalents are unchanged;
    only raw values move. The result is validated to stay inside 0..4095.
    """

    updated = deepcopy(dict(document))
    joints = updated["joints"]
    assert isinstance(joints, dict)
    for step in steps:
        joint = joints[step.joint_name]
        old, new = step.current_offset_raw, step.target_offset_raw
        joint["neutral_raw"] = shifted_raw(step.neutral_raw, old, new)
        joint["servo_homing_offset_raw"] = new
        joint["lerobot_homing_offset"] = new
        for field in ("supported_rest_minimum_raw", "supported_rest_maximum_raw"):
            if type(joint.get(field)) is int:
                joint[field] = shifted_raw(joint[field], old, new)
        if crosses_encoder_boundary(
            joint["neutral_raw"], joint["safe_min_delta_raw"], joint["safe_max_delta_raw"]
        ):
            raise CalibrationError(
                f"{step.joint_name} still crosses raw 0/4095 after centring; recalibrate it."
            )
    updated["servo_homing_offsets_written_at"] = (centred_at or datetime.now(UTC)).isoformat()
    return updated
