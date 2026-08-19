"""Tests for canonical extraction of ROS joint feedback."""

import pytest
from sensor_msgs.msg import JointState

from orion_motion.ros_state_reader import (
    JointStateError,
    joint_state_to_measured_state,
    require_fresh_measured_state,
)


JOINT_NAMES = (
    "base_yaw_joint",
    "shoulder_pitch_joint",
    "elbow_pitch_joint",
    "head_roll_joint",
    "head_pitch_joint",
)


def make_message():
    message = JointState()
    message.name = [
        "unrelated_joint",
        "head_pitch_joint",
        "elbow_pitch_joint",
        "base_yaw_joint",
        "head_roll_joint",
        "shoulder_pitch_joint",
    ]
    message.position = [9.0, 0.5, 0.3, 0.1, 0.4, 0.2]
    message.velocity = [0.0, 0.05, 0.03, 0.01, 0.04, 0.02]
    return message


def test_joint_state_is_reordered_by_semantic_name():
    measured = joint_state_to_measured_state(make_message(), JOINT_NAMES)

    assert measured.positions == pytest.approx((0.1, 0.2, 0.3, 0.4, 0.5))
    assert measured.velocities == pytest.approx((0.01, 0.02, 0.03, 0.04, 0.05))


def test_missing_required_joint_is_rejected():
    message = make_message()
    index = message.name.index("elbow_pitch_joint")
    del message.name[index]
    del message.position[index]
    del message.velocity[index]

    with pytest.raises(JointStateError, match="missing.*elbow_pitch_joint"):
        joint_state_to_measured_state(message, JOINT_NAMES)


def test_missing_velocity_feedback_is_rejected():
    message = make_message()
    message.velocity = []

    with pytest.raises(JointStateError, match="velocities are required"):
        joint_state_to_measured_state(message, JOINT_NAMES)


def test_duplicate_joint_name_is_rejected():
    message = make_message()
    message.name[-1] = "base_yaw_joint"

    with pytest.raises(JointStateError, match="duplicate"):
        joint_state_to_measured_state(message, JOINT_NAMES)


def test_recent_state_passes_freshness_check():
    measured = joint_state_to_measured_state(
        make_message(), JOINT_NAMES, received_at=10.0
    )

    require_fresh_measured_state(measured, 0.25, now=10.20)


def test_stale_state_is_rejected_before_execution():
    measured = joint_state_to_measured_state(
        make_message(), JOINT_NAMES, received_at=10.0
    )

    with pytest.raises(JointStateError, match="age.*exceeds"):
        require_fresh_measured_state(measured, 0.25, now=10.26)
