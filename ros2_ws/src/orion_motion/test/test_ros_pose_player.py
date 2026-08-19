"""Tests for resolving direct named-pose requests."""

from pathlib import Path

import pytest

from orion_motion.motion_validator import MotionValidationError
from orion_motion.ros_motion_player import duration_seconds, trajectory_to_message
from orion_motion.ros_pose_player import load_installed_pose_trajectory


PACKAGE_DIRECTORY = Path(__file__).parent.parent


def test_named_pose_resolves_all_five_joints_in_canonical_order():
    _, trajectory = load_installed_pose_trajectory(
        "attentive",
        1.25,
        0.20,
        package_share=PACKAGE_DIRECTORY,
    )

    assert trajectory.name == "go_to_pose:attentive"
    assert trajectory.joint_names == (
        "base_yaw_joint",
        "shoulder_pitch_joint",
        "elbow_pitch_joint",
        "head_roll_joint",
        "head_pitch_joint",
    )
    assert trajectory.keyframes[0].positions == pytest.approx(
        (-0.30, -0.10, -0.28, -0.65, -0.22)
    )


def test_named_pose_duration_and_hold_become_controller_points():
    _, trajectory = load_installed_pose_trajectory(
        "home",
        1.25,
        0.20,
        package_share=PACKAGE_DIRECTORY,
    )

    message = trajectory_to_message(trajectory)
    assert [duration_seconds(point.time_from_start) for point in message.points] == pytest.approx(
        [1.25, 1.45]
    )


def test_unknown_named_pose_is_rejected_before_playback():
    with pytest.raises(MotionValidationError, match="unknown pose 'missing'"):
        load_installed_pose_trajectory(
            "missing",
            1.0,
            0.0,
            package_share=PACKAGE_DIRECTORY,
        )
