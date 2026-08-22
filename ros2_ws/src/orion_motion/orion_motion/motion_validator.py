"""Validate simulator-independent Orion motion configuration."""

import math
from typing import Any


class MotionValidationError(ValueError):
    """Raised when loaded motion data violates Orion's data contract."""


# Check that the value is a dictionary.
def _require_mapping(value: Any, path: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise MotionValidationError(f"{path} must be a mapping")
    return value


# Check that the value is a usable integer or decimal number.
def _require_finite_number(value: Any, path: str) -> float | int:
    if (
        # Reject True and False because Python also treats them as integers.
        isinstance(value, bool)
        or not isinstance(value, (int, float))
        or not math.isfinite(value)
    ):
        raise MotionValidationError(f"{path} must be a finite number")
    return value


# Check that the file uses the expected format version.
def _require_format_version(
    data: dict[str, Any], path: str, *, expected: int = 1
) -> None:
    if (
        type(data.get("format_version")) is not int
        or data["format_version"] != expected
    ):
        raise MotionValidationError(
            f"{path}.format_version must be the integer {expected}"
        )


# Check that the file uses format version 1 and stores angles in radians.
def _require_format_header(data: dict[str, Any], path: str) -> None:
    _require_format_version(data, path)
    if data.get("units") != "radians":
        raise MotionValidationError(f"{path}.units must be 'radians'")


# List any missing or extra joint names in a clear error message.
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
    """Validate and return Orion's joint and dynamic-limit contract."""

    # Motion-limit files use format version 2.
    root = _require_mapping(data, "motion_limits")
    _require_format_version(root, "motion_limits", expected=2)

    expected_units = {
        "position": "radians",
        "velocity": "radians_per_second",
        "acceleration": "radians_per_second_squared",
        "jerk": "radians_per_second_cubed",
    }

    # Check the units used for position, speed, acceleration, and jerk.
    units = _require_mapping(root.get("units"), "motion_limits.units")
    if units != expected_units:
        raise MotionValidationError(
            "motion_limits.units must define radians and their time derivatives"
        )

    # These limits are approved for simulation only.
    if root.get("applicability") != "provisional_simulation_only":
        raise MotionValidationError(
            "motion_limits.applicability must be 'provisional_simulation_only'"
        )

    # Keep one clear joint order for all generated trajectories.
    joint_order = root.get("joint_order")
    if not isinstance(joint_order, list) or not joint_order:
        raise MotionValidationError("motion_limits.joint_order must be a non-empty list")
    if any(not isinstance(name, str) or not name for name in joint_order):
        raise MotionValidationError(
            "motion_limits.joint_order entries must be non-empty strings"
        )
    if len(set(joint_order)) != len(joint_order):
        raise MotionValidationError("motion_limits.joint_order must not contain duplicates")

    # Make sure every listed joint has limits and there are no extra joints.
    joints = _require_mapping(root.get("joints"), "motion_limits.joints")
    expected_names = set(joint_order)
    actual_names = set(joints)
    if actual_names != expected_names:
        difference = _describe_joint_difference(actual_names, expected_names)
        raise MotionValidationError(
            f"motion_limits.joints must match joint_order: {difference}"
        )

    # Check the position and movement limits for each joint.
    for joint_name in joint_order:
        limits = _require_mapping(
            joints[joint_name], f"motion_limits.joints.{joint_name}"
        )

        ranges: dict[str, tuple[float | int, float | int]] = {}
        for range_name in ("mechanical_position", "operational_position"):
            position_range = _require_mapping(
                limits.get(range_name),
                f"motion_limits.joints.{joint_name}.{range_name}",
            )
            lower = _require_finite_number(
                position_range.get("lower"),
                f"motion_limits.joints.{joint_name}.{range_name}.lower",
            )
            upper = _require_finite_number(
                position_range.get("upper"),
                f"motion_limits.joints.{joint_name}.{range_name}.upper",
            )
            if lower >= upper:
                raise MotionValidationError(
                    f"motion_limits.joints.{joint_name}.{range_name}.lower "
                    "must be less than upper"
                )
            ranges[range_name] = (lower, upper)

        # The normal working range must stay inside the mechanical range.
        mechanical_lower, mechanical_upper = ranges["mechanical_position"]
        operational_lower, operational_upper = ranges["operational_position"]
        if (
            operational_lower < mechanical_lower
            or operational_upper > mechanical_upper
        ):
            raise MotionValidationError(
                f"motion_limits.joints.{joint_name}.operational_position must "
                "be contained by mechanical_position"
            )

        # Speed, acceleration, jerk, and stopping limits must be above zero.
        for dynamic_name in (
            "max_velocity",
            "max_acceleration",
            "max_jerk",
            "max_cancel_deceleration",
        ):
            value = _require_finite_number(
                limits.get(dynamic_name),
                f"motion_limits.joints.{joint_name}.{dynamic_name}",
            )
            if value <= 0:
                raise MotionValidationError(
                    f"motion_limits.joints.{joint_name}.{dynamic_name} must be "
                    "greater than zero"
                )

    # The allowed starting speed cannot be negative.
    start_state = _require_mapping(
        root.get("start_state"), "motion_limits.start_state"
    )
    max_start_velocity = _require_finite_number(
        start_state.get("max_abs_velocity"),
        "motion_limits.start_state.max_abs_velocity",
    )
    if max_start_velocity < 0:
        raise MotionValidationError(
            "motion_limits.start_state.max_abs_velocity must not be negative"
        )

    return root


def validate_pose_library(
    data: Any, motion_limits: Any
) -> dict[str, Any]:
    """Validate and return a named-pose library against Orion's limits."""

    # Check the motion limits before using them to check poses.
    limits = validate_motion_limits(motion_limits)

    # Pose files must use format version 1 and radians.
    root = _require_mapping(data, "poses")
    _require_format_header(root, "poses")

    # The pose library must contain at least one pose.
    poses = _require_mapping(root.get("poses"), "poses.poses")
    if not poses:
        raise MotionValidationError("poses.poses must contain at least one named pose")

    # Every pose must use the joints listed in the motion limits.
    joint_order = limits["joint_order"]
    expected_names = set(joint_order)
    joint_limits = limits["joints"]

    # Check the name, description, and joint positions of each pose.
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
        # Each pose must give a position for every joint and no extra joints.
        actual_names = set(positions)
        if actual_names != expected_names:
            difference = _describe_joint_difference(actual_names, expected_names)
            raise MotionValidationError(
                f"poses.poses.{pose_name}.positions must contain exactly the "
                f"configured joints: {difference}"
            )

        # Each joint position must be a usable number inside its allowed range.
        for joint_name in joint_order:
            position = _require_finite_number(
                positions[joint_name],
                f"poses.poses.{pose_name}.positions.{joint_name}",
            )
            operational_range = joint_limits[joint_name]["operational_position"]
            lower = operational_range["lower"]
            upper = operational_range["upper"]
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

    # Motion files use format version 1.
    root = _require_mapping(data, "motion_definition")
    _require_format_version(root, "motion_definition")

    # Get the known poses so keyframes can be checked against them.
    poses_root = _require_mapping(pose_library, "pose_library")
    poses = _require_mapping(poses_root.get("poses"), "pose_library.poses")

    # Check the motion name and its optional description.
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

    # A motion must contain at least one keyframe.
    keyframes = motion.get("keyframes")
    if not isinstance(keyframes, list) or not keyframes:
        raise MotionValidationError(
            "motion_definition.motion.keyframes must be a non-empty list"
        )

    # Reject unknown fields, including misspelled field names.
    allowed_keyframe_fields = {"pose", "duration", "hold"}
    for index, keyframe_value in enumerate(keyframes):
        path = f"motion_definition.motion.keyframes[{index}]"
        keyframe = _require_mapping(keyframe_value, path)

        unexpected_fields = set(keyframe) - allowed_keyframe_fields
        if unexpected_fields:
            raise MotionValidationError(
                f"{path} contains unexpected fields: {sorted(unexpected_fields)}"
            )

        # Check that the keyframe names a pose that exists.
        pose_name = keyframe.get("pose")
        if not isinstance(pose_name, str) or not pose_name:
            raise MotionValidationError(f"{path}.pose must be a non-empty string")
        if pose_name not in poses:
            raise MotionValidationError(
                f"{path}.pose references unknown pose '{pose_name}'"
            )

        # Duration must be above zero. Hold time is optional and defaults to zero.
        duration = _require_finite_number(
            keyframe.get("duration"), f"{path}.duration"
        )
        if duration <= 0:
            raise MotionValidationError(f"{path}.duration must be greater than zero")

        hold = _require_finite_number(keyframe.get("hold", 0.0), f"{path}.hold")
        if hold < 0:
            raise MotionValidationError(f"{path}.hold must not be negative")

    return root
