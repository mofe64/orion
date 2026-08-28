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
YAW_MAX_ABS_DELTA_RAW = 1100  # About 96.7 degrees from neutral, with tolerance.
YAW_JOINTS = frozenset({"base_yaw_joint", "head_roll_joint"})


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
    lerobot_drive_mode: int
    lerobot_homing_offset: int
    lerobot_safe_range_min: int
    lerobot_safe_range_max: int


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
    """Reject missed joints, continuous-looking sweeps, and yaw over-rotation."""

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
        if capture.measured_span_raw > MAX_CAPTURE_SPAN_RAW:
            raise CalibrationError(
                f"{assignment.joint_name} covered {capture.measured_span_raw} raw steps "
                "(over 202.5 deg). Orion has no continuous joint; inspect the capture."
            )
        if assignment.joint_name in YAW_JOINTS and (
            abs(capture.measured_min_delta_raw) > YAW_MAX_ABS_DELTA_RAW
            or abs(capture.measured_max_delta_raw) > YAW_MAX_ABS_DELTA_RAW
        ):
            raise CalibrationError(
                f"{assignment.joint_name} exceeded the LeLamp cable-safe yaw window of about "
                "+/-90 deg from neutral. Inspect cable routing before retrying."
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
