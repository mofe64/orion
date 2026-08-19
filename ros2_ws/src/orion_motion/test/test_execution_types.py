"""Tests for backend-neutral motion execution records."""

from orion_motion.execution_types import ExecutionResult, ExecutionStatus


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
