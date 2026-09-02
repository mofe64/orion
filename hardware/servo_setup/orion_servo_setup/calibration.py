"""Software-only calibration capture for Orion's assembled STS3215 joints.

The reference LeLamp workflow writes a homing offset and position limits to
each servo.  Orion first records the equivalent values in a versioned file so
the physical measurements can be reviewed and rolled back before any EEPROM
calibration is changed.
"""

from __future__ import annotations

import json
import os
from collections.abc import Iterable, Mapping
from copy import deepcopy
from dataclasses import asdict, dataclass
from datetime import UTC, datetime
from pathlib import Path

from .provisioning import ORION_SERVO_ASSIGNMENTS, ServoAssignment, validate_assignments


ENCODER_RESOLUTION = 4096
HALF_TURN_RAW = ENCODER_RESOLUTION // 2
LELAMP_HOMING_TARGET_RAW = 2047
SAFE_MARGIN_RAW = 20  # About 1.76 degrees inside each measured endpoint.
MIN_CAPTURE_SPAN_RAW = 512  # 45 degrees; catches joints that were not swept.
MIN_EACH_SIDE_RAW = 128  # 11.25 degrees; proves both sides of neutral were sampled.
MAX_CAPTURE_SPAN_RAW = 2304  # 202.5 degrees; Orion joints are not continuous.
YAW_REFERENCE_LIMIT_RAW = 1024  # 90 degrees from neutral.
YAW_SAFE_LIMIT_RAW = YAW_REFERENCE_LIMIT_RAW - SAFE_MARGIN_RAW
YAW_JOINTS = frozenset({"base_yaw_joint", "head_roll_joint"})
SUPPORTED_REST_MINIMUM_JOINT = "shoulder_pitch_joint"
SUPPORTED_REST_JOINTS = frozenset(
    {SUPPORTED_REST_MINIMUM_JOINT, "elbow_pitch_joint"}
)
SUPPORTED_REST_MAX_EXTENSION_RAW = 128


class CalibrationError(RuntimeError):
    """Raised when captured measurements cannot form a safe calibration."""


@dataclass(frozen=True)
class JointRangeCapture:
    """Measured encoder travel expressed relative to the captured neutral."""

    assignment: ServoAssignment
    neutral_raw: int
    measured_min_delta_raw: int
    measured_max_delta_raw: int

    @property
    def measured_span_raw(self) -> int:
        return self.measured_max_delta_raw - self.measured_min_delta_raw


@dataclass(frozen=True)
class JointCalibration:
    """One joint's rollback-safe mapping and LeRobot-compatible equivalents."""

    servo_id: int
    joint_ref_name: str
    neutral_raw: int
    encoder_direction: int
    measured_min_delta_raw: int
    measured_max_delta_raw: int
    safe_min_delta_raw: int
    safe_max_delta_raw: int
    safe_min_degrees: float
    safe_max_degrees: float
    safety_cap_applied: bool
    lerobot_drive_mode: int
    lerobot_homing_offset: int
    lerobot_safe_range_min: int
    lerobot_safe_range_max: int


@dataclass(frozen=True)
class HardwareJointCalibration:
    """Minimal, validated calibration view used by torque-free setup tools."""

    joint_name: str
    servo_id: int
    neutral_raw: int
    encoder_direction: int
    safe_min_delta_raw: int
    safe_max_delta_raw: int


def load_hardware_calibration(path: Path) -> dict[str, HardwareJointCalibration]:
    """Load the active Orion calibration without interpreting any pose format."""

    path = path.expanduser()
    try:
        root = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise CalibrationError(f"Could not read calibration '{path}': {exc}") from exc
    if not isinstance(root, dict) or root.get("schema_version") != 1:
        raise CalibrationError("Calibration must use schema_version 1.")
    if (
        root.get("robot") != "orion"
        or root.get("servo_model") != "sts3215"
        or root.get("writes_servo_eeprom") is not False
    ):
        raise CalibrationError("Calibration is not an Orion STS3215 software calibration.")

    raw_joints = root.get("joints")
    expected = {item.joint_name for item in ORION_SERVO_ASSIGNMENTS}
    if not isinstance(raw_joints, dict) or set(raw_joints) != expected:
        raise CalibrationError("Calibration must contain Orion's five canonical joints.")

    result: dict[str, HardwareJointCalibration] = {}
    for assignment in ORION_SERVO_ASSIGNMENTS:
        raw = raw_joints[assignment.joint_name]
        if not isinstance(raw, dict):
            raise CalibrationError(f"Calibration {assignment.joint_name} must be a mapping.")
        fields = {
            field_name: raw.get(field_name)
            for field_name in (
                "servo_id",
                "neutral_raw",
                "encoder_direction",
                "safe_min_delta_raw",
                "safe_max_delta_raw",
            )
        }
        for field_name, value in fields.items():
            if type(value) is not int:
                raise CalibrationError(
                    f"{assignment.joint_name}.{field_name} must be an integer."
                )
        servo_id = fields["servo_id"]
        neutral = fields["neutral_raw"]
        direction = fields["encoder_direction"]
        safe_min = fields["safe_min_delta_raw"]
        safe_max = fields["safe_max_delta_raw"]
        if servo_id != assignment.servo_id:
            raise CalibrationError(
                f"{assignment.joint_name} calibration ID {servo_id} does not match ID "
                f"{assignment.servo_id}."
            )
        if not 0 <= neutral < ENCODER_RESOLUTION:
            raise CalibrationError(f"{assignment.joint_name} neutral is outside 0..4095.")
        if direction not in (-1, 1):
            raise CalibrationError(f"{assignment.joint_name} direction must be -1 or +1.")
        if not -HALF_TURN_RAW < safe_min < 0 < safe_max < HALF_TURN_RAW:
            raise CalibrationError(
                f"{assignment.joint_name} safe range must contain calibrated zero."
            )
        result[assignment.joint_name] = HardwareJointCalibration(
            joint_name=assignment.joint_name,
            servo_id=servo_id,
            neutral_raw=neutral,
            encoder_direction=direction,
            safe_min_delta_raw=safe_min,
            safe_max_delta_raw=safe_max,
        )
    return result


def accept_supported_rest_endpoint(
    document: Mapping[str, object],
    *,
    joint_name: str,
    raw_position: int,
) -> dict[str, object]:
    """Make a measured, mechanically supported rest endpoint commandable.

    Only Orion's shoulder and elbow pitch joints may use this exception. The
    live position must extend one existing safe endpoint and remain close to
    the measured range. The accepted endpoint intentionally has no inward
    margin; all other endpoints retain their existing margins.
    """

    if joint_name not in SUPPORTED_REST_JOINTS:
        allowed = ", ".join(sorted(SUPPORTED_REST_JOINTS))
        raise CalibrationError(f"Supported rest joint must be one of: {allowed}.")
    updated = deepcopy(dict(document))
    if (
        updated.get("schema_version") != 1
        or updated.get("robot") != "orion"
        or updated.get("servo_model") != "sts3215"
        or updated.get("encoder_resolution") != ENCODER_RESOLUTION
        or updated.get("writes_servo_eeprom") is not False
    ):
        raise CalibrationError(
            "Calibration is not an Orion STS3215 schema-version-1 file."
        )
    if type(raw_position) is not int or not 0 <= raw_position < ENCODER_RESOLUTION:
        raise CalibrationError(
            f"Supported rest raw position must be in 0..4095; got {raw_position!r}."
        )

    joints = updated.get("joints")
    if not isinstance(joints, dict):
        raise CalibrationError("Calibration joints must be a mapping.")
    joint = joints.get(joint_name)
    if not isinstance(joint, dict):
        raise CalibrationError(f"Calibration is missing {joint_name}.")
    neutral = joint.get("neutral_raw")
    measured_min = joint.get("measured_min_delta_raw")
    measured_max = joint.get("measured_max_delta_raw")
    safe_min = joint.get("safe_min_delta_raw")
    safe_max = joint.get("safe_max_delta_raw")
    for field_name, value in (
        ("neutral_raw", neutral),
        ("measured_min_delta_raw", measured_min),
        ("measured_max_delta_raw", measured_max),
        ("safe_min_delta_raw", safe_min),
        ("safe_max_delta_raw", safe_max),
    ):
        if type(value) is not int:
            raise CalibrationError(f"{joint_name}.{field_name} must be an integer.")

    delta = circular_delta(raw_position, neutral)
    if neutral + delta != raw_position:
        raise CalibrationError(
            "Supported rest crosses the raw encoder boundary; "
            "a direct LeRobot range value cannot represent it."
        )
    if safe_min <= delta <= safe_max:
        raise CalibrationError(
            f"Supported rest {delta:+d} is already inside the safe range."
        )

    if delta < safe_min:
        if delta < measured_min - SUPPORTED_REST_MAX_EXTENSION_RAW:
            raise CalibrationError(
                f"Supported rest {delta:+d} is more than "
                f"{SUPPORTED_REST_MAX_EXTENSION_RAW} counts beyond the measured minimum "
                f"{measured_min:+d}; repeat the main range calibration."
            )
        if delta <= -HALF_TURN_RAW or delta >= safe_max:
            raise CalibrationError("Supported rest does not form a valid bounded range.")
        endpoint = "minimum"
        joint["measured_min_delta_raw"] = min(measured_min, delta)
        joint["safe_min_delta_raw"] = delta
        joint["safe_min_degrees"] = delta * 360.0 / ENCODER_RESOLUTION
        joint["lerobot_safe_range_min"] = LELAMP_HOMING_TARGET_RAW + delta
    else:
        if delta > measured_max + SUPPORTED_REST_MAX_EXTENSION_RAW:
            raise CalibrationError(
                f"Supported rest {delta:+d} is more than "
                f"{SUPPORTED_REST_MAX_EXTENSION_RAW} counts beyond the measured maximum "
                f"{measured_max:+d}; repeat the main range calibration."
            )
        if delta >= HALF_TURN_RAW or delta <= safe_min:
            raise CalibrationError("Supported rest does not form a valid bounded range.")
        endpoint = "maximum"
        joint["measured_max_delta_raw"] = max(measured_max, delta)
        joint["safe_max_delta_raw"] = delta
        joint["safe_max_degrees"] = delta * 360.0 / ENCODER_RESOLUTION
        joint["lerobot_safe_range_max"] = LELAMP_HOMING_TARGET_RAW + delta

    joint[f"supported_rest_{endpoint}_raw"] = raw_position
    joint[f"supported_rest_{endpoint}_delta_raw"] = delta
    joint[f"supported_rest_{endpoint}_has_margin"] = False
    return updated


def accept_supported_rest_minimum(
    document: Mapping[str, object],
    *,
    raw_position: int,
) -> dict[str, object]:
    """Backward-compatible shoulder-minimum wrapper."""

    updated = accept_supported_rest_endpoint(
        document,
        joint_name=SUPPORTED_REST_MINIMUM_JOINT,
        raw_position=raw_position,
    )
    shoulder = updated["joints"][SUPPORTED_REST_MINIMUM_JOINT]  # type: ignore[index]
    if "supported_rest_minimum_delta_raw" not in shoulder:
        raise CalibrationError("Supported shoulder rest must be below calibrated zero.")
    return updated


def circular_delta(raw_position: int, neutral_raw: int) -> int:
    """Return the shortest signed 12-bit encoder delta from ``neutral_raw``.

    This makes a joint moving from raw 4090 to raw 10 appear as a small
    positive movement rather than a false movement of almost one full turn.
    """

    for label, value in (("raw position", raw_position), ("neutral", neutral_raw)):
        if not 0 <= value < ENCODER_RESOLUTION:
            raise ValueError(f"Encoder {label} must be in 0..4095; got {value}.")
    return (raw_position - neutral_raw + HALF_TURN_RAW) % ENCODER_RESOLUTION - HALF_TURN_RAW


def initialize_captures(
    neutral_positions: Mapping[str, int],
    assignments: Iterable[ServoAssignment] = ORION_SERVO_ASSIGNMENTS,
) -> dict[str, JointRangeCapture]:
    """Start every measured range at the captured neutral pose."""

    captures: dict[str, JointRangeCapture] = {}
    for assignment in validate_assignments(assignments):
        try:
            neutral = int(neutral_positions[assignment.joint_name])
        except KeyError as exc:
            raise CalibrationError(
                f"Neutral capture is missing {assignment.joint_name}."
            ) from exc
        if not 0 <= neutral < ENCODER_RESOLUTION:
            raise CalibrationError(
                f"{assignment.joint_name} returned invalid neutral raw value {neutral}."
            )
        captures[assignment.joint_name] = JointRangeCapture(assignment, neutral, 0, 0)
    return captures


def update_captures(
    captures: Mapping[str, JointRangeCapture],
    positions: Mapping[str, int],
) -> dict[str, JointRangeCapture]:
    """Extend measured ranges with one simultaneous raw-position sample."""

    updated: dict[str, JointRangeCapture] = {}
    for joint_name, capture in captures.items():
        try:
            raw_position = int(positions[joint_name])
        except KeyError as exc:
            raise CalibrationError(f"Range sample is missing {joint_name}.") from exc
        delta = circular_delta(raw_position, capture.neutral_raw)
        updated[joint_name] = JointRangeCapture(
            assignment=capture.assignment,
            neutral_raw=capture.neutral_raw,
            measured_min_delta_raw=min(capture.measured_min_delta_raw, delta),
            measured_max_delta_raw=max(capture.measured_max_delta_raw, delta),
        )
    return updated


def validate_captures(
    captures: Mapping[str, JointRangeCapture],
) -> tuple[JointRangeCapture, ...]:
    """Reject missed joints and ranges inconsistent with Orion's bounded joints."""

    expected_names = {item.joint_name for item in ORION_SERVO_ASSIGNMENTS}
    captured_names = set(captures)
    if captured_names != expected_names:
        missing = ", ".join(sorted(expected_names - captured_names)) or "none"
        unexpected = ", ".join(sorted(captured_names - expected_names)) or "none"
        raise CalibrationError(
            f"Calibration must contain Orion's five joints; missing: {missing}; "
            f"unexpected: {unexpected}."
        )

    validated: list[JointRangeCapture] = []
    for assignment in validate_assignments(ORION_SERVO_ASSIGNMENTS):
        capture = captures[assignment.joint_name]
        if capture.measured_span_raw < MIN_CAPTURE_SPAN_RAW:
            raise CalibrationError(
                f"{assignment.joint_name} covered only {capture.measured_span_raw} raw steps "
                f"(~{capture.measured_span_raw * 360 / ENCODER_RESOLUTION:.1f} deg). "
                "That joint was not swept through enough of its usable range."
            )
        if (
            capture.measured_min_delta_raw > -MIN_EACH_SIDE_RAW
            or capture.measured_max_delta_raw < MIN_EACH_SIDE_RAW
        ):
            raise CalibrationError(
                f"{assignment.joint_name} was not moved clearly to both sides of neutral. "
                "Sweep it in both directions from the zero/middle pose."
            )
        # Base yaw and head roll have a stricter commandable envelope applied
        # when the document is built below. Retain their measured endpoints
        # even when the physical sweep exceeds the generic bounded-joint span;
        # the dedicated cable-protection cap is the authoritative limit for
        # those two joints. Uncapped joints must still look non-continuous.
        if (
            assignment.joint_name not in YAW_JOINTS
            and capture.measured_span_raw > MAX_CAPTURE_SPAN_RAW
        ):
            raise CalibrationError(
                f"{assignment.joint_name} covered {capture.measured_span_raw} raw steps "
                "(over 202.5 deg). Orion has no continuous joint; inspect the capture."
            )
        if capture.measured_span_raw <= 2 * SAFE_MARGIN_RAW:
            raise CalibrationError(f"{assignment.joint_name} has no range after its safety margin.")
        validated.append(capture)
    return tuple(sorted(validated, key=lambda item: item.assignment.servo_id))


def build_calibration_document(
    captures: Mapping[str, JointRangeCapture],
    *,
    port: str,
    captured_at: datetime | None = None,
) -> dict[str, object]:
    """Build Orion's versioned JSON document without writing servo EEPROM."""

    joints: dict[str, dict[str, object]] = {}
    for capture in validate_captures(captures):
        safe_min = capture.measured_min_delta_raw + SAFE_MARGIN_RAW
        safe_max = capture.measured_max_delta_raw - SAFE_MARGIN_RAW
        safety_cap_applied = False
        if capture.assignment.joint_name in YAW_JOINTS:
            capped_min = max(safe_min, -YAW_SAFE_LIMIT_RAW)
            capped_max = min(safe_max, YAW_SAFE_LIMIT_RAW)
            safety_cap_applied = capped_min != safe_min or capped_max != safe_max
            safe_min, safe_max = capped_min, capped_max
        homing_offset = capture.neutral_raw - LELAMP_HOMING_TARGET_RAW
        calibration = JointCalibration(
            servo_id=capture.assignment.servo_id,
            joint_ref_name=capture.assignment.joint_ref_name,
            neutral_raw=capture.neutral_raw,
            encoder_direction=1,
            measured_min_delta_raw=capture.measured_min_delta_raw,
            measured_max_delta_raw=capture.measured_max_delta_raw,
            safe_min_delta_raw=safe_min,
            safe_max_delta_raw=safe_max,
            safe_min_degrees=safe_min * 360.0 / ENCODER_RESOLUTION,
            safe_max_degrees=safe_max * 360.0 / ENCODER_RESOLUTION,
            safety_cap_applied=safety_cap_applied,
            lerobot_drive_mode=0,
            lerobot_homing_offset=homing_offset,
            lerobot_safe_range_min=LELAMP_HOMING_TARGET_RAW + safe_min,
            lerobot_safe_range_max=LELAMP_HOMING_TARGET_RAW + safe_max,
        )
        joints[capture.assignment.joint_name] = asdict(calibration)

    timestamp = captured_at or datetime.now(UTC)
    return {
        "schema_version": 1,
        "robot": "orion",
        "servo_model": "sts3215",
        "encoder_resolution": ENCODER_RESOLUTION,
        "captured_at": timestamp.isoformat(),
        "port_at_capture": port,
        "writes_servo_eeprom": False,
        "direction_source": (
            "LeLamp follower reference uses drive_mode=0 for the same STS3215 mounting; "
            "validate against Orion URDF before trajectory control"
        ),
        "joints": joints,
    }


def write_calibration_file(document: Mapping[str, object], output_path: Path) -> Path | None:
    """Atomically save JSON, retaining a timestamped backup of an older file."""

    output_path = output_path.expanduser()
    output_path.parent.mkdir(parents=True, exist_ok=True)
    backup_path: Path | None = None
    if output_path.exists():
        stamp = datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")
        backup_path = output_path.with_name(
            f"{output_path.stem}.backup-{stamp}{output_path.suffix}"
        )
        suffix = 1
        while backup_path.exists():
            backup_path = output_path.with_name(
                f"{output_path.stem}.backup-{stamp}-{suffix}{output_path.suffix}"
            )
            suffix += 1
        output_path.replace(backup_path)

    temporary_path = output_path.with_suffix(f"{output_path.suffix}.tmp")
    try:
        temporary_path.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
        os.replace(temporary_path, output_path)
    except BaseException:
        temporary_path.unlink(missing_ok=True)
        if backup_path is not None and not output_path.exists():
            backup_path.replace(output_path)
        raise
    return backup_path
