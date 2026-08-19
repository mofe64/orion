"""Tests for converting resolved Orion motions to ROS trajectory messages."""

from pathlib import Path

import pytest

from orion_motion.motion_loader import load_yaml_file
from orion_motion.ros_motion_player import (
    duration_seconds,
    seconds_to_duration,
    trajectory_to_message,
)
from orion_motion.trajectory_builder import build_trajectory


PACKAGE_DIRECTORY = Path(__file__).parent.parent
CONFIG_DIRECTORY = PACKAGE_DIRECTORY / "config"
MOTIONS_DIRECTORY = PACKAGE_DIRECTORY / "motions"


def load_project_trajectory(relative_motion_path):
    return build_trajectory(
        load_yaml_file(MOTIONS_DIRECTORY / relative_motion_path),
        load_yaml_file(CONFIG_DIRECTORY / "poses.yaml"),
        load_yaml_file(CONFIG_DIRECTORY / "motion_limits.yaml"),
    )


def test_functional_motion_converts_arrival_and_hold_to_two_points():
    trajectory = load_project_trajectory("functional/look_at_left.yaml")

    message = trajectory_to_message(trajectory)

    assert message.joint_names == list(trajectory.joint_names)
    assert len(message.points) == 2
    assert list(message.points[0].positions) == list(
        trajectory.keyframes[0].positions
    )
    assert list(message.points[1].positions) == list(
        trajectory.keyframes[0].positions
    )
    assert duration_seconds(message.points[0].time_from_start) == pytest.approx(1.5)
    assert duration_seconds(message.points[1].time_from_start) == pytest.approx(2.0)


def test_expressive_motion_preserves_all_arrival_and_hold_times():
    trajectory = load_project_trajectory(
        "expressive/look_at_left_expressive.yaml"
    )

    message = trajectory_to_message(trajectory)

    actual_times = [
        duration_seconds(point.time_from_start) for point in message.points
    ]
    assert actual_times == pytest.approx(
        [0.25, 0.37, 0.82, 0.90, 1.65, 1.75, 2.0, 2.5]
    )
    assert list(message.points[-1].positions) == list(
        trajectory.keyframes[-1].positions
    )


def test_zero_hold_does_not_create_a_duplicate_point():
    trajectory = load_project_trajectory("functional/look_at_left.yaml")
    keyframe = trajectory.keyframes[0]
    no_hold_trajectory = type(trajectory)(
        name=trajectory.name,
        description=trajectory.description,
        joint_names=trajectory.joint_names,
        keyframes=(
            type(keyframe)(
                pose_name=keyframe.pose_name,
                positions=keyframe.positions,
                start_time=keyframe.start_time,
                arrival_time=keyframe.arrival_time,
                hold_until=keyframe.arrival_time,
            ),
        ),
        total_duration=keyframe.arrival_time,
    )

    message = trajectory_to_message(no_hold_trajectory)

    assert len(message.points) == 1


@pytest.mark.parametrize("invalid_seconds", [-1.0, float("inf"), float("nan")])
def test_duration_rejects_invalid_values(invalid_seconds):
    with pytest.raises(ValueError, match="finite and non-negative"):
        seconds_to_duration(invalid_seconds)
