"""Tests for Orion's simulator-independent motion validation."""

from copy import deepcopy
from pathlib import Path

import pytest

from orion_motion.motion_loader import load_yaml_file
from orion_motion.motion_validator import (
    MotionValidationError,
    validate_motion_definition,
    validate_motion_limits,
    validate_pose_library,
)


CONFIG_DIRECTORY = Path(__file__).parent.parent / "config"
MOTIONS_DIRECTORY = Path(__file__).parent.parent / "motions"


@pytest.fixture
def valid_limits():
    return load_yaml_file(CONFIG_DIRECTORY / "motion_limits.yaml")


@pytest.fixture
def valid_poses():
    return load_yaml_file(CONFIG_DIRECTORY / "poses.yaml")


@pytest.fixture
def valid_return_home():
    return load_yaml_file(MOTIONS_DIRECTORY / "functional" / "return_home.yaml")


def test_project_pose_library_is_valid(valid_poses, valid_limits):
    assert validate_pose_library(valid_poses, valid_limits) is valid_poses


def test_project_return_home_motion_is_valid(valid_return_home, valid_poses):
    assert (
        validate_motion_definition(valid_return_home, valid_poses)
        is valid_return_home
    )


@pytest.mark.parametrize(
    "motion_path",
    sorted(MOTIONS_DIRECTORY.rglob("*.yaml")),
    ids=lambda path: str(path.relative_to(MOTIONS_DIRECTORY)),
)
def test_every_project_motion_is_valid(motion_path, valid_poses):
    motion = load_yaml_file(motion_path)
    assert validate_motion_definition(motion, valid_poses) is motion


def test_motion_limits_reject_reversed_range(valid_limits):
    invalid_limits = deepcopy(valid_limits)
    invalid_limits["joints"]["elbow_pitch_joint"]["lower"] = 1.0
    invalid_limits["joints"]["elbow_pitch_joint"]["upper"] = -1.0

    with pytest.raises(MotionValidationError, match="lower must be less than upper"):
        validate_motion_limits(invalid_limits)


def test_pose_rejects_missing_joint(valid_poses, valid_limits):
    invalid_poses = deepcopy(valid_poses)
    del invalid_poses["poses"]["home"]["positions"]["head_pitch_joint"]

    with pytest.raises(MotionValidationError, match="missing.*head_pitch_joint"):
        validate_pose_library(invalid_poses, valid_limits)


@pytest.mark.parametrize("invalid_value", [True, float("inf"), float("nan")])
def test_pose_rejects_non_finite_number(
    invalid_value, valid_poses, valid_limits
):
    invalid_poses = deepcopy(valid_poses)
    invalid_poses["poses"]["home"]["positions"]["head_roll_joint"] = invalid_value

    with pytest.raises(MotionValidationError, match="must be a finite number"):
        validate_pose_library(invalid_poses, valid_limits)


def test_pose_rejects_position_outside_limits(valid_poses, valid_limits):
    invalid_poses = deepcopy(valid_poses)
    invalid_poses["poses"]["home"]["positions"]["head_pitch_joint"] = 3.0

    with pytest.raises(MotionValidationError, match="outside.*radians"):
        validate_pose_library(invalid_poses, valid_limits)


def test_pose_rejects_wrong_units(valid_poses, valid_limits):
    invalid_poses = deepcopy(valid_poses)
    invalid_poses["units"] = "degrees"

    with pytest.raises(MotionValidationError, match="units must be 'radians'"):
        validate_pose_library(invalid_poses, valid_limits)


def test_motion_rejects_unknown_pose(valid_return_home, valid_poses):
    invalid_motion = deepcopy(valid_return_home)
    invalid_motion["motion"]["keyframes"][0]["pose"] = "missing_pose"

    with pytest.raises(MotionValidationError, match="unknown pose 'missing_pose'"):
        validate_motion_definition(invalid_motion, valid_poses)


@pytest.mark.parametrize("invalid_duration", [0.0, -1.0])
def test_motion_rejects_non_positive_duration(
    invalid_duration, valid_return_home, valid_poses
):
    invalid_motion = deepcopy(valid_return_home)
    invalid_motion["motion"]["keyframes"][0]["duration"] = invalid_duration

    with pytest.raises(MotionValidationError, match="duration must be greater than zero"):
        validate_motion_definition(invalid_motion, valid_poses)


def test_motion_rejects_negative_hold(valid_return_home, valid_poses):
    invalid_motion = deepcopy(valid_return_home)
    invalid_motion["motion"]["keyframes"][0]["hold"] = -0.1

    with pytest.raises(MotionValidationError, match="hold must not be negative"):
        validate_motion_definition(invalid_motion, valid_poses)


def test_motion_rejects_unexpected_keyframe_field(valid_return_home, valid_poses):
    invalid_motion = deepcopy(valid_return_home)
    invalid_motion["motion"]["keyframes"][0]["duraton"] = 2.0

    with pytest.raises(MotionValidationError, match="unexpected fields.*duraton"):
        validate_motion_definition(invalid_motion, valid_poses)
