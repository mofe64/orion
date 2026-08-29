"""Tests for resolving symbolic motions into backend-neutral trajectories."""

from copy import deepcopy
from pathlib import Path

import pytest

from orion_motion.motion_loader import load_yaml_file
from orion_motion.motion_validator import MotionValidationError
from orion_motion.trajectory_builder import build_trajectory


PACKAGE_DIRECTORY = Path(__file__).parent.parent
CONFIG_DIRECTORY = PACKAGE_DIRECTORY / "config"
MOTIONS_DIRECTORY = PACKAGE_DIRECTORY / "motions"


@pytest.fixture
def project_limits():
    return load_yaml_file(CONFIG_DIRECTORY / "motion_limits.yaml")


@pytest.fixture
def project_poses():
    return load_yaml_file(CONFIG_DIRECTORY / "poses.yaml")


def test_builds_left_motion_in_canonical_joint_order(project_poses, project_limits):
    motion = load_yaml_file(MOTIONS_DIRECTORY / "functional" / "look_at_left.yaml")

    trajectory = build_trajectory(motion, project_poses, project_limits)

    assert trajectory.name == "look_at_left"
    assert trajectory.joint_names == (
        "base_yaw_joint",
        "shoulder_pitch_joint",
        "elbow_pitch_joint",
        "head_roll_joint",
        "head_pitch_joint",
    )
    assert len(trajectory.keyframes) == 1
    keyframe = trajectory.keyframes[0]
    assert keyframe.pose_name == "look_left"
    assert keyframe.positions == (-1.228, -0.10, -0.28, -0.65, -0.22)
    assert keyframe.start_time == pytest.approx(0.0)
    assert keyframe.arrival_time == pytest.approx(1.5)
    assert keyframe.hold_until == pytest.approx(2.0)
    assert trajectory.total_duration == pytest.approx(2.0)


def test_accumulates_multi_keyframe_durations_and_holds(
    project_poses, project_limits
):
    motion = {
        "format_version": 1,
        "motion": {
            "name": "timing_example",
            "keyframes": [
                {"pose": "attentive", "duration": 0.4, "hold": 0.2},
                {"pose": "look_right", "duration": 1.1, "hold": 0.3},
            ],
        },
    }

    trajectory = build_trajectory(motion, project_poses, project_limits)

    first, second = trajectory.keyframes
    assert (first.start_time, first.arrival_time, first.hold_until) == pytest.approx(
        (0.0, 0.4, 0.6)
    )
    assert (second.start_time, second.arrival_time, second.hold_until) == pytest.approx(
        (0.6, 1.7, 2.0)
    )
    assert trajectory.total_duration == pytest.approx(2.0)


def test_validates_before_resolving(project_poses, project_limits):
    motion = {
        "format_version": 1,
        "motion": {
            "name": "invalid_example",
            "keyframes": [
                {"pose": "missing_pose", "duration": 1.0},
            ],
        },
    }

    with pytest.raises(MotionValidationError, match="unknown pose 'missing_pose'"):
        build_trajectory(motion, project_poses, project_limits)


def test_does_not_mutate_source_data(project_poses, project_limits):
    motion = load_yaml_file(MOTIONS_DIRECTORY / "functional" / "look_at_right.yaml")
    original_motion = deepcopy(motion)
    original_poses = deepcopy(project_poses)
    original_limits = deepcopy(project_limits)

    build_trajectory(motion, project_poses, project_limits)

    assert motion == original_motion
    assert project_poses == original_poses
    assert project_limits == original_limits


def test_expressive_right_uses_mirrored_derived_poses(
    project_poses, project_limits
):
    motion = load_yaml_file(
        MOTIONS_DIRECTORY / "expressive" / "look_at_right_expressive.yaml"
    )

    trajectory = build_trajectory(motion, project_poses, project_limits)

    assert [keyframe.pose_name for keyframe in trajectory.keyframes] == [
        "look_right_anticipation",
        "look_right_lean",
        "look_right_overshoot",
        "look_right",
    ]
    assert len(trajectory.keyframes) == 4
    assert trajectory.keyframes[-1].positions == (
        1.54,
        -0.10,
        -0.28,
        -0.65,
        -0.22,
    )
    assert trajectory.total_duration == pytest.approx(4.12)
