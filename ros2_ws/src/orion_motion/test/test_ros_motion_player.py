"""Tests for converting resolved Orion motions to ROS trajectory messages."""

from copy import deepcopy
from pathlib import Path

import pytest

from orion_motion.motion_loader import load_yaml_file
from orion_motion.ros_motion_player import (
    duration_seconds,
    seconds_to_duration,
    trajectory_to_message,
)
from orion_motion.trajectory_builder import build_trajectory
from orion_motion.trajectory_generator import generate_trajectory


PACKAGE_DIRECTORY = Path(__file__).parent.parent
CONFIG_DIRECTORY = PACKAGE_DIRECTORY / "config"
MOTIONS_DIRECTORY = PACKAGE_DIRECTORY / "motions"


def load_project_trajectory(relative_motion_path, *, allow_aggressive=False):
    poses = load_yaml_file(CONFIG_DIRECTORY / "poses.yaml")
    limits = load_yaml_file(CONFIG_DIRECTORY / "motion_limits.yaml")
    requested = build_trajectory(
        load_yaml_file(MOTIONS_DIRECTORY / relative_motion_path),
        poses,
        limits,
    )
    if allow_aggressive:
        limits = deepcopy(limits)
        for joint_limits in limits["joints"].values():
            joint_limits["max_velocity"] = 10_000.0
            joint_limits["max_acceleration"] = 10_000.0
            joint_limits["max_jerk"] = 10_000.0
    start = tuple(
        poses["poses"]["attentive"]["positions"][joint_name]
        for joint_name in requested.joint_names
    )
    return generate_trajectory(requested, start, (0.0,) * 5, limits)


def test_functional_motion_includes_measured_start_arrival_and_hold():
    trajectory = load_project_trajectory("functional/look_at_left.yaml")

    message = trajectory_to_message(trajectory)

    assert message.joint_names == list(trajectory.joint_names)
    assert len(message.points) == 3
    assert list(message.points[0].positions) == list(
        trajectory.points[0].positions
    )
    assert list(message.points[1].positions) == list(
        trajectory.points[1].positions
    )
    assert list(message.points[2].positions) == list(
        trajectory.points[2].positions
    )
    assert [
        duration_seconds(point.time_from_start) for point in message.points
    ] == pytest.approx([0.0, 1.5, 2.0])


def test_expressive_motion_preserves_all_arrival_and_hold_times():
    trajectory = load_project_trajectory(
        "expressive/look_at_left_expressive.yaml",
        allow_aggressive=True,
    )

    message = trajectory_to_message(trajectory)

    actual_times = [
        duration_seconds(point.time_from_start) for point in message.points
    ]
    assert actual_times == pytest.approx(
        [0.0, 0.25, 0.37, 0.82, 0.90, 1.65, 1.75, 2.0, 2.5]
    )
    assert list(message.points[-1].positions) == list(
        trajectory.points[-1].positions
    )


def test_zero_hold_does_not_create_a_duplicate_point():
    poses = load_yaml_file(CONFIG_DIRECTORY / "poses.yaml")
    limits = load_yaml_file(CONFIG_DIRECTORY / "motion_limits.yaml")
    motion = load_yaml_file(MOTIONS_DIRECTORY / "functional/look_at_left.yaml")
    motion["motion"]["keyframes"][0]["hold"] = 0.0
    requested = build_trajectory(motion, poses, limits)
    start = tuple(
        poses["poses"]["attentive"]["positions"][joint_name]
        for joint_name in requested.joint_names
    )
    no_hold_trajectory = generate_trajectory(
        requested, start, (0.0,) * 5, limits
    )

    message = trajectory_to_message(no_hold_trajectory)

    assert len(message.points) == 2


def test_ros_points_include_velocity_and_acceleration_fields():
    trajectory = load_project_trajectory("functional/return_home.yaml")

    message = trajectory_to_message(trajectory)

    for source, point in zip(trajectory.points, message.points, strict=True):
        assert list(point.velocities) == list(source.velocities)
        assert list(point.accelerations) == list(source.accelerations)


@pytest.mark.parametrize("invalid_seconds", [-1.0, float("inf"), float("nan")])
def test_duration_rejects_invalid_values(invalid_seconds):
    with pytest.raises(ValueError, match="finite and non-negative"):
        seconds_to_duration(invalid_seconds)
