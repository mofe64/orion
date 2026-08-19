"""Validate simulator-independent Orion motion configuration."""

import math
from typing import Any


class MotionValidationError(ValueError):
    """Raised when loaded motion data violates Orion's data contract."""


def _require_mapping(value: Any, path: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise MotionValidationError(f"{path} must be a mapping")
    return value


def _require_finite_number(value: Any, path: str) -> float | int:
    if (
        isinstance(value, bool)
        or not isinstance(value, (int, float))
        or not math.isfinite(value)
    ):
        raise MotionValidationError(f"{path} must be a finite number")
    return value


def _require_format_version(data: dict[str, Any], path: str) -> None:
    if type(data.get("format_version")) is not int or data["format_version"] != 1:
        raise MotionValidationError(f"{path}.format_version must be the integer 1")


def _require_format_header(data: dict[str, Any], path: str) -> None:
    _require_format_version(data, path)
    if data.get("units") != "radians":
        raise MotionValidationError(f"{path}.units must be 'radians'")


def _describe_joint_difference(
    actual_names: set[str], expected_names: set[str]
) -> str:
    details = []
    missing = sorted(expected_names - actual_names)
    unexpected = sorted(actual_names - expected_names)
    if missing:
        details.append(f"missing {missing}")
    if unexpected:
        details.append(f"unexpected {unexpected}")
    return "; ".join(details)


def validate_motion_limits(data: Any) -> dict[str, Any]:
    """Validate and return Orion's joint-order and position-limit contract."""

    root = _require_mapping(data, "motion_limits")
    _require_format_header(root, "motion_limits")

    if root.get("limit_kind") != "mechanical_position":
        raise MotionValidationError(
            "motion_limits.limit_kind must be 'mechanical_position'"
        )

    joint_order = root.get("joint_order")
    if not isinstance(joint_order, list) or not joint_order:
        raise MotionValidationError("motion_limits.joint_order must be a non-empty list")
    if any(not isinstance(name, str) or not name for name in joint_order):
        raise MotionValidationError(
            "motion_limits.joint_order entries must be non-empty strings"
        )
    if len(set(joint_order)) != len(joint_order):
        raise MotionValidationError("motion_limits.joint_order must not contain duplicates")

    joints = _require_mapping(root.get("joints"), "motion_limits.joints")
    expected_names = set(joint_order)
    actual_names = set(joints)
    if actual_names != expected_names:
        difference = _describe_joint_difference(actual_names, expected_names)
        raise MotionValidationError(
            f"motion_limits.joints must match joint_order: {difference}"
        )

    for joint_name in joint_order:
        limits = _require_mapping(
            joints[joint_name], f"motion_limits.joints.{joint_name}"
        )
        lower = _require_finite_number(
            limits.get("lower"), f"motion_limits.joints.{joint_name}.lower"
        )
        upper = _require_finite_number(
            limits.get("upper"), f"motion_limits.joints.{joint_name}.upper"
        )
        if lower >= upper:
            raise MotionValidationError(
                f"motion_limits.joints.{joint_name}.lower must be less than upper"
            )

    return root


def validate_pose_library(
    data: Any, motion_limits: Any
) -> dict[str, Any]:
    """Validate and return a named-pose library against Orion's limits."""

    limits = validate_motion_limits(motion_limits)
    root = _require_mapping(data, "poses")
    _require_format_header(root, "poses")

    poses = _require_mapping(root.get("poses"), "poses.poses")
    if not poses:
        raise MotionValidationError("poses.poses must contain at least one named pose")

    joint_order = limits["joint_order"]
    expected_names = set(joint_order)
    joint_limits = limits["joints"]

    for pose_name, pose_value in poses.items():
        if not isinstance(pose_name, str) or not pose_name:
            raise MotionValidationError("pose names must be non-empty strings")

        pose = _require_mapping(pose_value, f"poses.poses.{pose_name}")
        description = pose.get("description")
        if description is not None and not isinstance(description, str):
            raise MotionValidationError(
                f"poses.poses.{pose_name}.description must be a string"
            )

        positions = _require_mapping(
            pose.get("positions"), f"poses.poses.{pose_name}.positions"
        )
        actual_names = set(positions)
        if actual_names != expected_names:
            difference = _describe_joint_difference(actual_names, expected_names)
            raise MotionValidationError(
                f"poses.poses.{pose_name}.positions must contain exactly the "
                f"configured joints: {difference}"
            )

        for joint_name in joint_order:
            position = _require_finite_number(
                positions[joint_name],
                f"poses.poses.{pose_name}.positions.{joint_name}",
            )
            lower = joint_limits[joint_name]["lower"]
            upper = joint_limits[joint_name]["upper"]
            if not lower <= position <= upper:
                raise MotionValidationError(
                    f"poses.poses.{pose_name}.positions.{joint_name}={position} "
                    f"is outside [{lower}, {upper}] radians"
                )

    return root


def validate_motion_definition(
    data: Any, pose_library: Any
) -> dict[str, Any]:
    """Validate one named, pose-referenced keyframe motion."""

    root = _require_mapping(data, "motion_definition")
    _require_format_version(root, "motion_definition")

    poses_root = _require_mapping(pose_library, "pose_library")
    poses = _require_mapping(poses_root.get("poses"), "pose_library.poses")

    motion = _require_mapping(root.get("motion"), "motion_definition.motion")
    name = motion.get("name")
    if not isinstance(name, str) or not name:
        raise MotionValidationError(
            "motion_definition.motion.name must be a non-empty string"
        )

    description = motion.get("description")
    if description is not None and not isinstance(description, str):
        raise MotionValidationError(
            "motion_definition.motion.description must be a string"
        )

    keyframes = motion.get("keyframes")
    if not isinstance(keyframes, list) or not keyframes:
        raise MotionValidationError(
            "motion_definition.motion.keyframes must be a non-empty list"
        )

    allowed_keyframe_fields = {"pose", "duration", "hold"}
    for index, keyframe_value in enumerate(keyframes):
        path = f"motion_definition.motion.keyframes[{index}]"
        keyframe = _require_mapping(keyframe_value, path)

        unexpected_fields = set(keyframe) - allowed_keyframe_fields
        if unexpected_fields:
            raise MotionValidationError(
                f"{path} contains unexpected fields: {sorted(unexpected_fields)}"
            )

        pose_name = keyframe.get("pose")
        if not isinstance(pose_name, str) or not pose_name:
            raise MotionValidationError(f"{path}.pose must be a non-empty string")
        if pose_name not in poses:
            raise MotionValidationError(
                f"{path}.pose references unknown pose '{pose_name}'"
            )

        duration = _require_finite_number(
            keyframe.get("duration"), f"{path}.duration"
        )
        if duration <= 0:
            raise MotionValidationError(f"{path}.duration must be greater than zero")

        hold = _require_finite_number(keyframe.get("hold", 0.0), f"{path}.hold")
        if hold < 0:
            raise MotionValidationError(f"{path}.hold must not be negative")

    return root
