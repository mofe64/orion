"""Capture and persist Orion's torque-free mechanically stable rest pose."""

from __future__ import annotations

import math
import os
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any

import yaml

from .calibration import ENCODER_RESOLUTION, circular_delta
from .archived.pose_execution import (
    HardwareJointCalibration,
    STEPS_PER_RADIAN,
)
from .provisioning import ORION_SERVO_ASSIGNMENTS


REST_POSE_NAME = "rest"
REST_DESCRIPTION = (
    "Captured torque-free mechanical rest pose; verify stability again after hardware changes."
)
STABILITY_DURATION_SECONDS = 5.0
STABILITY_TOLERANCE_RAW = 10  # About 0.88 degrees.


class RestCaptureError(RuntimeError):
    """Raised when a candidate rest pose is unsafe, unstable, or cannot be saved."""


def load_operational_ranges(path: Path) -> dict[str, tuple[float, float]]:
    """Load the shared ROS pose-library position contract."""

    path = path.expanduser()
    try:
        root = yaml.safe_load(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, yaml.YAMLError) as exc:
        raise RestCaptureError(f"Could not read motion limits '{path}': {exc}") from exc
    joints = root.get("joints") if isinstance(root, dict) else None
    if not isinstance(joints, dict):
        raise RestCaptureError("Motion limits must contain a joints mapping.")

    ranges: dict[str, tuple[float, float]] = {}
    for assignment in ORION_SERVO_ASSIGNMENTS:
        joint = joints.get(assignment.joint_name)
        operational = joint.get("operational_position") if isinstance(joint, dict) else None
        if not isinstance(operational, dict):
            raise RestCaptureError(
                f"Motion limits are missing {assignment.joint_name}.operational_position."
            )
        lower = operational.get("lower")
        upper = operational.get("upper")
        if (
            isinstance(lower, bool)
            or isinstance(upper, bool)
            or not isinstance(lower, (int, float))
            or not isinstance(upper, (int, float))
            or not math.isfinite(float(lower))
            or not math.isfinite(float(upper))
            or float(lower) >= float(upper)
        ):
            raise RestCaptureError(
                f"Motion limits for {assignment.joint_name} must be a finite increasing range."
            )
        ranges[assignment.joint_name] = (float(lower), float(upper))
    return ranges


def positions_to_rest_angles(
    positions: Mapping[str, int],
    calibration: Mapping[str, HardwareJointCalibration],
    operational_ranges: Mapping[str, tuple[float, float]],
) -> dict[str, float]:
    """Convert one raw, safe, non-wrapped capture into shared pose radians."""

    expected = {assignment.joint_name for assignment in ORION_SERVO_ASSIGNMENTS}
    if set(positions) != expected:
        raise RestCaptureError("Rest capture must contain Orion's five canonical joints.")

    result: dict[str, float] = {}
    for assignment in ORION_SERVO_ASSIGNMENTS:
        name = assignment.joint_name
        raw = positions[name]
        if type(raw) is not int or not 0 <= raw < ENCODER_RESOLUTION:
            raise RestCaptureError(f"{name} returned invalid raw position {raw!r}.")
        joint = calibration[name]
        delta = circular_delta(raw, joint.neutral_raw)
        if not joint.safe_min_delta_raw <= delta <= joint.safe_max_delta_raw:
            raise RestCaptureError(
                f"{name} rest delta {delta:+d} is outside calibrated "
                f"[{joint.safe_min_delta_raw}, {joint.safe_max_delta_raw}]."
            )
        if joint.neutral_raw + delta != raw:
            raise RestCaptureError(
                f"{name} rest position crosses the raw 0/4095 boundary; choose a rest "
                "position closer to calibrated zero."
            )

        angle = delta / (STEPS_PER_RADIAN * joint.encoder_direction)
        lower, upper = operational_ranges[name]
        if not lower <= angle <= upper:
            raise RestCaptureError(
                f"{name} rest angle {angle:+.4f} rad is outside the shared pose-library "
                f"range [{lower}, {upper}]."
            )
        # Eight decimal places round-trips to the same encoder step.
        result[name] = round(angle, 8)
    return result


def validate_rest_stability(
    reference: Mapping[str, int],
    samples: Sequence[Mapping[str, int]],
    *,
    tolerance_raw: int = STABILITY_TOLERANCE_RAW,
) -> dict[str, int]:
    """Reject a torque-off pose if any encoder drifts beyond the allowed window."""

    if not samples:
        raise RestCaptureError("The rest stability check did not collect any samples.")
    maximum_drift = {name: 0 for name in reference}
    for sample in samples:
        if set(sample) != set(reference):
            raise RestCaptureError("A stability sample did not contain all five joints.")
        for name, initial in reference.items():
            drift = abs(circular_delta(int(sample[name]), int(initial)))
            maximum_drift[name] = max(maximum_drift[name], drift)
    unstable = {name: drift for name, drift in maximum_drift.items() if drift > tolerance_raw}
    if unstable:
        details = ", ".join(f"{name}={drift} steps" for name, drift in unstable.items())
        raise RestCaptureError(
            f"Candidate rest pose moved with torque off ({details}); choose a more stable pose."
        )
    return maximum_drift


def write_rest_pose(
    pose_path: Path,
    positions_radians: Mapping[str, float],
    *,
    replace: bool = False,
) -> None:
    """Atomically add the captured rest pose to the shared YAML library."""

    pose_path = pose_path.expanduser()
    try:
        root: Any = yaml.safe_load(pose_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, yaml.YAMLError) as exc:
        raise RestCaptureError(f"Could not read pose library '{pose_path}': {exc}") from exc
    if not isinstance(root, dict) or root.get("format_version") != 1:
        raise RestCaptureError("Pose library must use format_version 1.")
    if root.get("units") != "radians":
        raise RestCaptureError("Pose library units must be radians.")
    poses = root.get("poses")
    if not isinstance(poses, dict):
        raise RestCaptureError("Pose library must contain a poses mapping.")
    if REST_POSE_NAME in poses and not replace:
        raise RestCaptureError("Pose 'rest' already exists; rerun with --replace to overwrite it.")

    poses[REST_POSE_NAME] = {
        "description": REST_DESCRIPTION,
        "positions": dict(positions_radians),
    }
    temporary_path = pose_path.with_suffix(f"{pose_path.suffix}.tmp")
    try:
        temporary_path.write_text(
            yaml.safe_dump(root, sort_keys=False, allow_unicode=True), encoding="utf-8"
        )
        os.replace(temporary_path, pose_path)
    except BaseException:
        temporary_path.unlink(missing_ok=True)
        raise
