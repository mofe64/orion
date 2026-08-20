"""Deterministic lifecycle tests for Orion's ROS action adapter."""

from copy import deepcopy
from pathlib import Path
from types import SimpleNamespace
import time

import pytest
from control_msgs.action import FollowJointTrajectory
from action_msgs.msg import GoalStatus

from orion_motion.execution_types import ExecutionResult, ExecutionStatus
from orion_motion.motion_loader import load_yaml_file
from orion_motion.motion_validator import MotionValidationError
from orion_motion.ros_motion_player import (
    GoalCancellation,
    LatestMotionRequestQueue,
    execute_motion_queue,
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
        self.wait_calls = 0

    def wait_for_server(self, *, timeout_sec):
        self.wait_calls += 1
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


def wrapped_result(
    error_code,
    error_string="",
    *,
    status=GoalStatus.STATUS_SUCCEEDED,
):
    result = FollowJointTrajectory.Result()
    result.error_code = error_code
    result.error_string = error_string
    return SimpleNamespace(result=result, status=status)


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
        "goal_settle_duration",
        "goal_settle_timeout",
        "result_timeout_factor",
        "result_timeout_margin",
        "cancel_response_timeout",
        "stop_confirmation_timeout",
        "stop_confirmation_duration",
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
    settled_calls = []

    def settled_waiter(node, joint_names, target_positions, **kwargs):
        settled_calls.append((tuple(joint_names), tuple(target_positions), kwargs))
        return MeasuredJointState(
            positions=tuple(target_positions),
            velocities=(0.0,) * len(joint_names),
            received_at=10.2,
        )

    result = send_trajectory_goal(
        FakeNode(),
        validated,
        state,
        policy,
        server_timeout=3.0,
        action_client=client,
        spin_until_complete=recording_spin(timeouts),
        monotonic=lambda: 10.1,
        settled_state_waiter=settled_waiter,
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
    assert len(settled_calls) == 1
    assert settled_calls[0][2] == {
        "maximum_position_error": pytest.approx(0.05),
        "maximum_velocity": pytest.approx(0.05),
        "stable_duration": pytest.approx(0.25),
        "timeout": pytest.approx(2.0),
    }
    assert result.metrics.settling_time == pytest.approx(0.0)
    assert result.metrics.final_position_errors == pytest.approx((0.0,) * 5)
    assert result.metrics.final_velocities == pytest.approx((0.0,) * 5)


def test_success_result_without_sustained_settling_is_failure():
    validated, state, policy = project_execution_inputs()
    handle = FakeGoalHandle(
        FakeFuture(wrapped_result(FollowJointTrajectory.Result.SUCCESSFUL))
    )

    def fail_to_settle(*args, **kwargs):
        from orion_motion.ros_state_reader import JointStateError

        raise JointStateError("still moving")

    result = send_trajectory_goal(
        FakeNode(),
        validated,
        state,
        policy,
        server_timeout=3.0,
        action_client=FakeActionClient(handle),
        spin_until_complete=recording_spin([]),
        monotonic=lambda: 10.1,
        settled_state_waiter=fail_to_settle,
    )

    assert result.status is ExecutionStatus.SETTLING_FAILED
    assert "still moving" in result.message


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
        FakeFuture(
            wrapped_result(
                error_code,
                "tolerance failure",
                status=GoalStatus.STATUS_ABORTED,
            )
        )
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
        stopped_state_waiter=lambda *args, **kwargs: state,
    )

    assert result.status is ExecutionStatus.TIMED_OUT
    assert result.cancel_requested
    assert result.stop_confirmed
    assert handle.cancel_calls == 1
    assert timeouts == pytest.approx([3.0, 11.5, 1.0, 1.0])


def test_repeated_cancel_requests_share_one_controller_request():
    cancellation = GoalCancellation()
    handle = FakeGoalHandle(FakeFuture())
    cancellation.attach(handle)

    first = cancellation.request(ExecutionStatus.PREEMPTED)
    second = cancellation.request(ExecutionStatus.PREEMPTED)
    third = cancellation.request(ExecutionStatus.CANCELLED)

    assert first is second is third
    assert handle.cancel_calls == 1
    assert cancellation.reason is ExecutionStatus.CANCELLED


def test_cancelled_action_requires_and_records_stopped_confirmation():
    validated, state, policy = project_execution_inputs()
    handle = FakeGoalHandle(
        FakeFuture(
            wrapped_result(
                FollowJointTrajectory.Result.SUCCESSFUL,
                status=GoalStatus.STATUS_CANCELED,
            )
        )
    )
    cancellation = GoalCancellation()
    cancellation.request(ExecutionStatus.CANCELLED)

    result = send_trajectory_goal(
        FakeNode(),
        validated,
        state,
        policy,
        server_timeout=3.0,
        action_client=FakeActionClient(handle),
        spin_until_complete=recording_spin([]),
        monotonic=lambda: 10.1,
        cancellation=cancellation,
        stopped_state_waiter=lambda *args, **kwargs: state,
    )

    assert result.status is ExecutionStatus.CANCELLED
    assert result.cancel_requested
    assert result.stop_confirmed
    assert handle.cancel_calls == 1


def test_cancelled_action_records_stopping_time_and_joint_distance():
    validated, state, policy = project_execution_inputs()
    feedback = make_feedback(validated.trajectory.joint_names)
    handle = FakeGoalHandle(
        FakeFuture(
            wrapped_result(
                FollowJointTrajectory.Result.SUCCESSFUL,
                status=GoalStatus.STATUS_CANCELED,
            )
        )
    )
    cancellation = GoalCancellation()

    def request_after_feedback(sample):
        cancellation.request(ExecutionStatus.CANCELLED)

    stopped = MeasuredJointState(
        positions=(0.92,) * 5,
        velocities=(0.0,) * 5,
        received_at=time.monotonic(),
    )
    result = send_trajectory_goal(
        FakeNode(),
        validated,
        state,
        policy,
        server_timeout=3.0,
        action_client=FakeActionClient(handle, feedback=feedback),
        spin_until_complete=recording_spin([]),
        monotonic=lambda: 10.1,
        cancellation=cancellation,
        stopped_state_waiter=lambda *args, **kwargs: stopped,
        feedback_observer=request_after_feedback,
    )

    assert result.status is ExecutionStatus.CANCELLED
    assert result.metrics.cancellation_stopping_time >= 0.0
    assert result.metrics.cancellation_stopping_distances == pytest.approx(
        (0.02,) * 5
    )


def test_latest_request_preempts_once_and_regenerates_from_fresh_state():
    limits = load_yaml_file(CONFIG_DIRECTORY / "motion_limits.yaml")
    poses = load_yaml_file(CONFIG_DIRECTORY / "poses.yaml")

    def requested(name):
        return build_trajectory(
            load_yaml_file(MOTIONS_DIRECTORY / "functional" / f"{name}.yaml"),
            poses,
            limits,
        )

    first = requested("look_at_left")
    replaced = requested("return_home")
    latest = requested("look_at_right")
    queue = LatestMotionRequestQueue()
    queue.submit(first)
    state_reads = []
    sent = []
    cancel_handles = []

    def state_reader(node, joint_names, *, timeout):
        offset = -0.1 * len(state_reads)
        positions = tuple(
            poses["poses"]["attentive"]["positions"][name]
            for name in joint_names
        )
        positions = (positions[0] + offset, *positions[1:])
        state = MeasuredJointState(
            positions=positions,
            velocities=(0.0,) * len(joint_names),
            received_at=10.0 + len(state_reads),
        )
        state_reads.append(state)
        return state

    def goal_sender(
        node,
        validated,
        start_state,
        policy,
        *,
        server_timeout,
        cancellation,
    ):
        sent.append(
            (
                validated.trajectory.name,
                validated.trajectory.points[0].positions,
            )
        )
        if len(sent) == 1:
            handle = FakeGoalHandle(FakeFuture())
            cancel_handles.append(handle)
            cancellation.attach(handle)
            queue.submit(replaced)
            queue.submit(latest)
            return ExecutionResult(
                motion_name=validated.trajectory.name,
                backend="test",
                status=ExecutionStatus.PREEMPTED,
                message="replaced",
                cancel_requested=True,
                stop_confirmed=True,
            )
        return ExecutionResult(
            motion_name=validated.trajectory.name,
            backend="test",
            status=ExecutionStatus.SUCCEEDED,
            message="done",
        )

    _, _, policy = project_execution_inputs()
    results = execute_motion_queue(
        FakeNode(),
        queue,
        PACKAGE_DIRECTORY,
        policy,
        state_timeout=3.0,
        server_timeout=3.0,
        state_reader=state_reader,
        goal_sender=goal_sender,
    )

    assert [result.status for result in results] == [
        ExecutionStatus.PREEMPTED,
        ExecutionStatus.SUCCEEDED,
    ]
    assert [name for name, _ in sent] == ["look_at_left", "look_at_right"]
    assert sent[0][1] == state_reads[0].positions
    assert sent[1][1] == state_reads[1].positions
    assert sent[0][1] != sent[1][1]
    assert cancel_handles[0].cancel_calls == 1


def test_queue_waits_for_action_server_before_reading_fresh_state():
    validated, _, policy = project_execution_inputs()
    requested = build_trajectory(
        load_yaml_file(MOTIONS_DIRECTORY / "functional/look_at_left.yaml"),
        load_yaml_file(CONFIG_DIRECTORY / "poses.yaml"),
        load_yaml_file(CONFIG_DIRECTORY / "motion_limits.yaml"),
    )
    queue = LatestMotionRequestQueue()
    queue.submit(requested)
    client = FakeActionClient(
        FakeGoalHandle(
            FakeFuture(wrapped_result(FollowJointTrajectory.Result.SUCCESSFUL))
        )
    )

    def state_reader(node, joint_names, *, timeout):
        assert client.wait_calls == 1
        return MeasuredJointState(
            positions=validated.trajectory.points[0].positions,
            velocities=(0.0,) * len(joint_names),
            received_at=time.monotonic(),
        )

    def goal_sender(*args, **kwargs):
        return send_trajectory_goal(
            *args,
            spin_until_complete=recording_spin([]),
            settled_state_waiter=lambda node, names, targets, **unused: (
                MeasuredJointState(
                    positions=tuple(targets),
                    velocities=(0.0,) * len(names),
                    received_at=time.monotonic(),
                )
            ),
            **kwargs,
        )

    results = execute_motion_queue(
        FakeNode(),
        queue,
        PACKAGE_DIRECTORY,
        policy,
        state_timeout=3.0,
        server_timeout=3.0,
        state_reader=state_reader,
        goal_sender=goal_sender,
        action_client=client,
    )

    assert results[-1].status is ExecutionStatus.SUCCEEDED
    assert client.wait_calls == 2


def test_user_cancel_discards_pending_replacement_and_overrides_preemption():
    limits = load_yaml_file(CONFIG_DIRECTORY / "motion_limits.yaml")
    poses = load_yaml_file(CONFIG_DIRECTORY / "poses.yaml")
    replacement = build_trajectory(
        load_yaml_file(MOTIONS_DIRECTORY / "functional/look_at_right.yaml"),
        poses,
        limits,
    )
    queue = LatestMotionRequestQueue()
    cancellation = GoalCancellation()
    handle = FakeGoalHandle(FakeFuture())
    cancellation.attach(handle)
    queue.set_active(cancellation)

    queue.submit(replacement)
    queue.cancel()

    assert queue.take_latest() is None
    assert cancellation.reason is ExecutionStatus.CANCELLED
    assert handle.cancel_calls == 1


def test_cancel_during_server_discovery_prevents_state_read_and_goal_send():
    limits = load_yaml_file(CONFIG_DIRECTORY / "motion_limits.yaml")
    poses = load_yaml_file(CONFIG_DIRECTORY / "poses.yaml")
    requested = build_trajectory(
        load_yaml_file(MOTIONS_DIRECTORY / "functional/look_at_left.yaml"),
        poses,
        limits,
    )
    queue = LatestMotionRequestQueue()
    queue.submit(requested)

    class CancellingClient(FakeActionClient):
        def wait_for_server(self, *, timeout_sec):
            self.wait_calls += 1
            queue.cancel()
            return True

    client = CancellingClient(None)

    def unexpected_state_read(*args, **kwargs):
        raise AssertionError("state must not be read after early cancellation")

    _, _, policy = project_execution_inputs()
    results = execute_motion_queue(
        FakeNode(),
        queue,
        PACKAGE_DIRECTORY,
        policy,
        state_timeout=3.0,
        server_timeout=3.0,
        state_reader=unexpected_state_read,
        action_client=client,
    )

    assert [result.status for result in results] == [ExecutionStatus.CANCELLED]
    assert client.send_calls == 0


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
