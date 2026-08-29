"""Tests for backend-neutral motion execution records."""

import pytest

from orion_motion.execution_types import (
    ExecutionFeedback,
    ExecutionResult,
    ExecutionStatus,
    JointExecutionState,
    execution_metrics_from_feedback,
    execution_result_data,
)


def test_only_succeeded_status_reports_success():
    successful = ExecutionResult(
        motion_name="look_at_left",
        backend="test",
        status=ExecutionStatus.SUCCEEDED,
        message="done",
    )
    timed_out = ExecutionResult(
        motion_name="look_at_left",
        backend="test",
        status=ExecutionStatus.TIMED_OUT,
        message="deadline",
        cancel_requested=True,
    )

    assert successful.succeeded
    assert not timed_out.succeeded


def test_cancel_result_distinguishes_request_from_confirmed_stop():
    cancelled = ExecutionResult(
        motion_name="look_at_left",
        backend="test",
        status=ExecutionStatus.CANCELLED,
        message="stopped",
        cancel_requested=True,
        stop_confirmed=True,
    )

    assert cancelled.cancel_requested
    assert cancelled.stop_confirmed
    assert not cancelled.succeeded


def test_feedback_metrics_preserve_per_joint_tracking_and_final_state():
    samples = []
    for time_from_start, errors, velocities in (
        (0.5, (0.10, -0.20), (0.30, -0.40)),
        (1.0, (-0.15, 0.05), (0.01, -0.02)),
    ):
        state = JointExecutionState(
            positions=(0.0, 0.0),
            velocities=velocities,
            accelerations=(0.0, 0.0),
            time_from_start=time_from_start,
        )
        error = JointExecutionState(
            positions=errors,
            velocities=(0.0, 0.0),
            accelerations=(0.0, 0.0),
            time_from_start=time_from_start,
        )
        samples.append(
            ExecutionFeedback(
                timestamp=time_from_start,
                joint_names=("first", "second"),
                desired=state,
                actual=state,
                error=error,
            )
        )

    metrics = execution_metrics_from_feedback(samples)

    assert metrics.maximum_position_errors == pytest.approx((0.15, 0.20))
    assert metrics.final_position_errors == pytest.approx((-0.15, 0.05))
    assert metrics.final_velocities == pytest.approx((0.01, -0.02))


def test_execution_result_data_converts_enum_for_json():
    result = ExecutionResult(
        motion_name="look_at_left",
        backend="native_test",
        status=ExecutionStatus.SUCCEEDED,
        message="done",
    )

    assert execution_result_data(result)["status"] == "succeeded"
