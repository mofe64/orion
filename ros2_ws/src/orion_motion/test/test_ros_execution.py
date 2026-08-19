"""Deterministic lifecycle tests for Orion's ROS action adapter."""

from copy import deepcopy
from pathlib import Path
from types import SimpleNamespace

import pytest
from control_msgs.action import FollowJointTrajectory

from orion_motion.execution_types import ExecutionStatus
from orion_motion.motion_loader import load_yaml_file
from orion_motion.motion_validator import MotionValidationError
from orion_motion.ros_motion_player import (
    execution_policy_from_data,
    send_trajectory_goal,
)
from orion_motion.ros_state_reader import MeasuredJointState
from orion_motion.trajectory_builder import build_trajectory
from orion_motion.trajectory_generator import generate_trajectory
from orion_motion.trajectory_validator import require_valid_trajectory


PACKAGE_DIRECTORY = Path(__file__).parent.parent
CONFIG_DIRECTORY = PACKAGE_DIRECTORY / "config"
MOTIONS_DIRECTORY = PACKAGE_DIRECTORY / "motions"


class FakeLogger:
    def __init__(self):
        self.messages = []

    def info(self, message):
        self.messages.append(("info", message))

    def error(self, message):
        self.messages.append(("error", message))


class FakeNode:
    def __init__(self):
        self.logger = FakeLogger()

    def get_logger(self):
        return self.logger


class FakeFuture:
    def __init__(self, result=None, *, done=True):
        self._result = result
        self._done = done

    def done(self):
        return self._done

    def result(self):
        return self._result


class FakeGoalHandle:
    def __init__(self, result_future, *, accepted=True):
        self.accepted = accepted
        self.result_future = result_future
        self.cancel_calls = 0

    def get_result_async(self):
        return self.result_future

    def cancel_goal_async(self):
        self.cancel_calls += 1
        return FakeFuture(SimpleNamespace())


class FakeActionClient:
    def __init__(self, goal_handle, *, feedback=None, server_available=True):
        self.goal_handle = goal_handle
        self.feedback = feedback
        self.server_available = server_available
        self.sent_goal = None
        self.send_calls = 0

    def wait_for_server(self, *, timeout_sec):
        return self.server_available

    def send_goal_async(self, goal, *, feedback_callback):
        self.sent_goal = goal
        self.send_calls += 1
        if self.feedback is not None:
            feedback_callback(SimpleNamespace(feedback=self.feedback))
        return FakeFuture(self.goal_handle)


def project_execution_inputs():
    limits = load_yaml_file(CONFIG_DIRECTORY / "motion_limits.yaml")
    poses = load_yaml_file(CONFIG_DIRECTORY / "poses.yaml")
    requested = build_trajectory(
        load_yaml_file(MOTIONS_DIRECTORY / "functional/look_at_left.yaml"),
        poses,
        limits,
    )
    start_positions = tuple(
        poses["poses"]["attentive"]["positions"][joint_name]
        for joint_name in requested.joint_names
    )
    generated = generate_trajectory(
        requested,
        start_positions,
        (0.0,) * len(start_positions),
        limits,
    )
    validated = require_valid_trajectory(
        generated,
        limits,
        load_yaml_file(CONFIG_DIRECTORY / "forbidden_regions.yaml"),
    )
    state = MeasuredJointState(
        positions=start_positions,
        velocities=(0.0,) * len(start_positions),
        received_at=10.0,
    )
    policy = execution_policy_from_data(
        load_yaml_file(CONFIG_DIRECTORY / "execution_policy.yaml")
    )
    return validated, state, policy


def make_feedback(joint_names):
    feedback = FollowJointTrajectory.Feedback()
    feedback.header.stamp.sec = 12
    feedback.header.stamp.nanosec = 250_000_000
    feedback.joint_names = list(joint_names)
    for point, positions in (
        (feedback.desired, (1.0,) * 5),
        (feedback.actual, (0.9,) * 5),
        (feedback.error, (0.1,) * 5),
    ):
        point.positions = list(positions)
        point.velocities = [0.2] * 5
        point.accelerations = [0.3] * 5
        point.time_from_start.sec = 1
        point.time_from_start.nanosec = 500_000_000
    return feedback


def wrapped_result(error_code, error_string=""):
    result = FollowJointTrajectory.Result()
    result.error_code = error_code
    result.error_string = error_string
    return SimpleNamespace(result=result)


def recording_spin(timeouts):
    def spin(node, future, *, timeout_sec):
        timeouts.append(timeout_sec)

    return spin


@pytest.mark.parametrize(
    "field",
    [
        "max_state_age",
        "path_position_tolerance",
        "goal_position_tolerance",
        "stopped_velocity_tolerance",
        "goal_time_tolerance",
        "result_timeout_factor",
        "result_timeout_margin",
        "cancel_response_timeout",
    ],
)
def test_execution_policy_requires_positive_finite_thresholds(field):
    policy_data = load_yaml_file(CONFIG_DIRECTORY / "execution_policy.yaml")
    invalid = deepcopy(policy_data)
    invalid[field] = 0.0

    with pytest.raises(MotionValidationError, match=field):
        execution_policy_from_data(invalid)


def test_execution_policy_requires_version_and_provisional_label():
    policy_data = load_yaml_file(CONFIG_DIRECTORY / "execution_policy.yaml")
    wrong_version = deepcopy(policy_data)
    wrong_version["format_version"] = 2
    wrong_label = deepcopy(policy_data)
    wrong_label["applicability"] = "physical_hardware"

    with pytest.raises(MotionValidationError, match="integer 1"):
        execution_policy_from_data(wrong_version)
    with pytest.raises(MotionValidationError, match="applicability"):
        execution_policy_from_data(wrong_label)


def test_success_preserves_feedback_and_applies_explicit_tolerances():
    validated, state, policy = project_execution_inputs()
    result_future = FakeFuture(
        wrapped_result(FollowJointTrajectory.Result.SUCCESSFUL)
    )
    handle = FakeGoalHandle(result_future)
    client = FakeActionClient(
        handle,
        feedback=make_feedback(validated.trajectory.joint_names),
    )
    timeouts = []

    result = send_trajectory_goal(
        FakeNode(),
        validated,
        state,
        policy,
        server_timeout=3.0,
        action_client=client,
        spin_until_complete=recording_spin(timeouts),
        monotonic=lambda: 10.1,
    )

    assert result.status is ExecutionStatus.SUCCEEDED
    assert result.succeeded
    assert result.backend == "ros2_control"
    assert result.feedback[0].timestamp == pytest.approx(12.25)
    assert result.feedback[0].desired.positions == pytest.approx((1.0,) * 5)
    assert result.feedback[0].actual.positions == pytest.approx((0.9,) * 5)
    assert result.feedback[0].error.positions == pytest.approx((0.1,) * 5)
    assert result.feedback[0].actual.velocities == pytest.approx((0.2,) * 5)
    assert result.feedback[0].error.accelerations == pytest.approx((0.3,) * 5)
    assert result.feedback[0].desired.time_from_start == pytest.approx(1.5)
    assert len(client.sent_goal.path_tolerance) == 5
    assert client.sent_goal.path_tolerance[0].position == pytest.approx(0.20)
    assert client.sent_goal.goal_tolerance[0].position == pytest.approx(0.05)
    assert client.sent_goal.goal_tolerance[0].velocity == pytest.approx(0.05)
    assert timeouts == pytest.approx([3.0, 11.5])


@pytest.mark.parametrize(
    "error_code,expected_status",
    [
        (
            FollowJointTrajectory.Result.PATH_TOLERANCE_VIOLATED,
            ExecutionStatus.PATH_TOLERANCE_VIOLATED,
        ),
        (
            FollowJointTrajectory.Result.GOAL_TOLERANCE_VIOLATED,
            ExecutionStatus.GOAL_TOLERANCE_VIOLATED,
        ),
    ],
)
def test_controller_tolerance_failures_remain_distinguishable(
    error_code, expected_status
):
    validated, state, policy = project_execution_inputs()
    handle = FakeGoalHandle(
        FakeFuture(wrapped_result(error_code, "tolerance failure"))
    )

    result = send_trajectory_goal(
        FakeNode(),
        validated,
        state,
        policy,
        server_timeout=3.0,
        action_client=FakeActionClient(handle),
        spin_until_complete=recording_spin([]),
        monotonic=lambda: 10.1,
    )

    assert result.status is expected_status
    assert result.backend_error_code == error_code
    assert not result.succeeded


def test_result_deadline_requests_cancellation():
    validated, state, policy = project_execution_inputs()
    handle = FakeGoalHandle(FakeFuture(done=False))
    timeouts = []

    result = send_trajectory_goal(
        FakeNode(),
        validated,
        state,
        policy,
        server_timeout=3.0,
        action_client=FakeActionClient(handle),
        spin_until_complete=recording_spin(timeouts),
        monotonic=lambda: 10.1,
    )

    assert result.status is ExecutionStatus.TIMED_OUT
    assert result.cancel_requested
    assert handle.cancel_calls == 1
    assert timeouts == pytest.approx([3.0, 11.5, 1.0])


def test_stale_start_is_rejected_without_sending_goal():
    validated, state, policy = project_execution_inputs()
    handle = FakeGoalHandle(
        FakeFuture(wrapped_result(FollowJointTrajectory.Result.SUCCESSFUL))
    )
    client = FakeActionClient(handle)

    result = send_trajectory_goal(
        FakeNode(),
        validated,
        state,
        policy,
        server_timeout=3.0,
        action_client=client,
        spin_until_complete=recording_spin([]),
        monotonic=lambda: 10.3,
    )

    assert result.status is ExecutionStatus.REJECTED
    assert "age" in result.message
    assert client.send_calls == 0


def test_unavailable_server_returns_bounded_timeout():
    validated, state, policy = project_execution_inputs()
    client = FakeActionClient(None, server_available=False)

    result = send_trajectory_goal(
        FakeNode(),
        validated,
        state,
        policy,
        server_timeout=3.0,
        action_client=client,
        spin_until_complete=recording_spin([]),
        monotonic=lambda: 10.1,
    )

    assert result.status is ExecutionStatus.TIMED_OUT
    assert not result.cancel_requested
    assert client.send_calls == 0
