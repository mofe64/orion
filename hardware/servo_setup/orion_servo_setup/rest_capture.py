"""Capture and persist Orion's torque-free mechanically stable rest pose."""

from __future__ import annotations

import math
import os
import re
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any

import yaml

from .calibration import ENCODER_RESOLUTION, HardwareJointCalibration, circular_delta
from .provisioning import ORION_SERVO_ASSIGNMENTS


REST_POSE_NAME = "rest"
REST_DESCRIPTION = (
    "Captured torque-free mechanical rest pose; verify stability again after hardware changes."
)
STABILITY_DURATION_SECONDS = 5.0
STABILITY_TOLERANCE_RAW = 10  # About 0.88 degrees.
STEPS_PER_RADIAN = ENCODER_RESOLUTION / (2.0 * math.pi)


class RestCaptureError(RuntimeError):
    """Raised when a candidate rest pose is unsafe, unstable, or cannot be saved."""


def positions_to_rest_angles(
    positions: Mapping[str, int],
    calibration: Mapping[str, HardwareJointCalibration],
) -> dict[str, float]:
    """Convert one raw capture using calibration as the sole position authority."""

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
        source = pose_path.read_text(encoding="utf-8")
        root: Any = yaml.safe_load(source)
    except (OSError, UnicodeError, yaml.YAMLError) as exc:
        raise RestCaptureError(f"Could not read pose library '{pose_path}': {exc}") from exc
    if not isinstance(root, dict) or root.get("format_version") != 2:
        raise RestCaptureError("Pose library must use format_version 2 (v2 required).")
    if root.get("units") != "radians":
        raise RestCaptureError("Pose library units must be radians.")
    poses = root.get("poses")
    if not isinstance(poses, dict):
        raise RestCaptureError("Pose library must contain a poses mapping.")
    if REST_POSE_NAME in poses and not replace:
        raise RestCaptureError("Pose 'rest' already exists; rerun with --replace to overwrite it.")

    expected_joints = tuple(item.joint_name for item in ORION_SERVO_ASSIGNMENTS)
    if set(positions_radians) != set(expected_joints):
        raise RestCaptureError("Rest pose must contain Orion's five canonical joints.")
    rest_lines = [
        "  rest:",
        f"    description: {REST_DESCRIPTION}",
        "    tags: [shutdown_only, mechanical_rest]",
        "    default_lighting: off",
        "    positions:",
    ]
    for joint_name in expected_joints:
        value = positions_radians[joint_name]
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise RestCaptureError(f"Rest value for {joint_name} must be numeric.")
        numeric_value = float(value)
        if not math.isfinite(numeric_value):
            raise RestCaptureError(f"Rest value for {joint_name} must be finite.")
        rest_lines.append(f"      {joint_name}: {numeric_value!r}")
    rest_block = "\n".join(rest_lines) + "\n\n"

    rest_match = re.search(r"(?m)^  rest:\s*$", source)
    if rest_match is not None:
        next_pose = re.search(r"(?m)^  [A-Za-z][A-Za-z0-9_]*:\s*$", source[rest_match.end():])
        end = len(source) if next_pose is None else rest_match.end() + next_pose.start()
        updated_source = source[:rest_match.start()] + rest_block + source[end:]
    else:
        poses_match = re.search(r"(?m)^poses:\s*$", source)
        if poses_match is None:
            raise RestCaptureError("Pose library must contain a poses mapping.")
        insertion = poses_match.end()
        updated_source = source[:insertion] + "\n" + rest_block + source[insertion + 1:]

    temporary_path = pose_path.with_suffix(f"{pose_path.suffix}.tmp")
    try:
        temporary_path.write_text(updated_source, encoding="utf-8")
        os.replace(temporary_path, pose_path)
    except BaseException:
        temporary_path.unlink(missing_ok=True)
        raise
