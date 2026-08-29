"""Resolve validated Orion motion definitions into timed joint targets."""

from dataclasses import dataclass
from typing import Any

from orion_motion.motion_validator import (
    validate_motion_definition,
    validate_motion_limits,
    validate_pose_library,
)


@dataclass(frozen=True)
class ResolvedKeyframe:
    """One complete joint target and its absolute timing information."""

    pose_name: str
    positions: tuple[float, ...]
    start_time: float
    arrival_time: float
    hold_until: float


@dataclass(frozen=True)
class ResolvedTrajectory:
    """A backend-neutral sequence of complete, ordered joint targets."""

    name: str
    description: str | None
    joint_names: tuple[str, ...]
    keyframes: tuple[ResolvedKeyframe, ...]
    total_duration: float


def build_trajectory(
    motion_definition: Any,
    pose_library: Any,
    motion_limits: Any,
) -> ResolvedTrajectory:
    """Validate and resolve a symbolic motion into absolute timed keyframes.

    Durations describe travel time from the preceding state or keyframe.
    Holds begin on arrival and delay the start of the following keyframe.
    The initial physical joint state is intentionally not part of this data
    transformation; an execution backend supplies that measured state.
    """

    limits = validate_motion_limits(motion_limits)
    poses_root = validate_pose_library(pose_library, limits)
    motion_root = validate_motion_definition(motion_definition, poses_root)

    joint_names = tuple(limits["joint_order"])
    poses = poses_root["poses"]
    motion = motion_root["motion"]

    elapsed = 0.0
    resolved_keyframes: list[ResolvedKeyframe] = []

    for keyframe in motion["keyframes"]:
        pose_name = keyframe["pose"]
        duration = float(keyframe["duration"])
        hold = float(keyframe.get("hold", 0.0))
        arrival_time = elapsed + duration
        hold_until = arrival_time + hold
        positions = tuple(
            float(poses[pose_name]["positions"][joint_name])
            for joint_name in joint_names
        )

        resolved_keyframes.append(
            ResolvedKeyframe(
                pose_name=pose_name,
                positions=positions,
                start_time=elapsed,
                arrival_time=arrival_time,
                hold_until=hold_until,
            )
        )
        elapsed = hold_until

    return ResolvedTrajectory(
        name=motion["name"],
        description=motion.get("description"),
        joint_names=joint_names,
        keyframes=tuple(resolved_keyframes),
        total_duration=elapsed,
    )


def build_pose_trajectory(
    pose_name: str,
    duration: float,
    pose_library: Any,
    motion_limits: Any,
    *,
    hold: float = 0.0,
) -> ResolvedTrajectory:
    """Resolve one named pose through the normal motion-validation path.

    A direct pose request is represented internally as a one-keyframe motion.
    This keeps pose playback backend-neutral and gives it exactly the same
    validation, joint ordering, and timing semantics as a stored motion file.
    """

    motion_definition = {
        "format_version": 1,
        "motion": {
            "name": f"go_to_pose:{pose_name}",
            "description": f"Move to the named pose '{pose_name}'.",
            "keyframes": [
                {
                    "pose": pose_name,
                    "duration": duration,
                    "hold": hold,
                }
            ],
        },
    }
    return build_trajectory(motion_definition, pose_library, motion_limits)
