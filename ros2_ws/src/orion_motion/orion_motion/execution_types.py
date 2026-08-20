"""Backend-neutral records for Orion motion execution."""

from __future__ import annotations

from dataclasses import asdict, dataclass
from enum import Enum
from typing import Any, Sequence


class ExecutionStatus(str, Enum):
    """Terminal outcome preserved across execution backends."""

    SUCCEEDED = "succeeded"
    REJECTED = "rejected"
    INVALID_GOAL = "invalid_goal"
    INVALID_JOINTS = "invalid_joints"
    OLD_HEADER_TIMESTAMP = "old_header_timestamp"
    PATH_TOLERANCE_VIOLATED = "path_tolerance_violated"
    GOAL_TOLERANCE_VIOLATED = "goal_tolerance_violated"
    CANCELLED = "cancelled"
    PREEMPTED = "preempted"
    TIMED_OUT = "timed_out"
    SETTLING_FAILED = "settling_failed"
    UNSAFE_STABILITY = "unsafe_stability"
    FAILED = "failed"


@dataclass(frozen=True)
class JointExecutionState:
    """One desired, actual, or error joint state from an executor."""

    positions: tuple[float, ...]
    velocities: tuple[float, ...]
    accelerations: tuple[float, ...]
    time_from_start: float


@dataclass(frozen=True)
class ExecutionFeedback:
    """One time-aligned desired-versus-measured execution observation."""

    timestamp: float
    joint_names: tuple[str, ...]
    desired: JointExecutionState
    actual: JointExecutionState
    error: JointExecutionState


@dataclass(frozen=True)
class ExecutionMetrics:
    """Optional measured completion and stability evidence."""

    maximum_position_errors: tuple[float, ...] = ()
    final_position_errors: tuple[float, ...] = ()
    final_velocities: tuple[float, ...] = ()
    settling_time: float | None = None
    cancellation_stopping_time: float | None = None
    cancellation_stopping_distances: tuple[float, ...] = ()
    maximum_base_translation: float | None = None
    maximum_base_tilt: float | None = None
    maximum_base_height_change: float | None = None
    longest_contact_loss: float | None = None


@dataclass(frozen=True)
class ExecutionResult:
    """Terminal execution outcome with all feedback retained for analysis."""

    motion_name: str
    backend: str
    status: ExecutionStatus
    message: str
    feedback: tuple[ExecutionFeedback, ...] = ()
    backend_error_code: int | None = None
    cancel_requested: bool = False
    stop_confirmed: bool = False
    metrics: ExecutionMetrics | None = None

    @property
    def succeeded(self) -> bool:
        """Return whether execution reached the backend's success state."""

        return self.status is ExecutionStatus.SUCCEEDED


def execution_metrics_from_feedback(
    feedback: Sequence[ExecutionFeedback],
) -> ExecutionMetrics:
    """Summarize time-aligned desired and measured controller feedback."""

    if not feedback:
        return ExecutionMetrics()

    joint_names = feedback[0].joint_names
    joint_count = len(joint_names)
    maximum_errors = [0.0] * joint_count
    for sample_index, sample in enumerate(feedback):
        if sample.joint_names != joint_names:
            raise ValueError(
                f"feedback sample {sample_index} changed joint order"
            )
        for field_name, values in (
            ("error.positions", sample.error.positions),
            ("actual.velocities", sample.actual.velocities),
        ):
            if len(values) != joint_count:
                raise ValueError(
                    f"feedback sample {sample_index} {field_name} must "
                    f"contain {joint_count} values"
                )
        for index, error in enumerate(sample.error.positions):
            maximum_errors[index] = max(maximum_errors[index], abs(error))

    final = feedback[-1]
    return ExecutionMetrics(
        maximum_position_errors=tuple(maximum_errors),
        final_position_errors=tuple(final.error.positions),
        final_velocities=tuple(final.actual.velocities),
    )


def execution_result_data(result: ExecutionResult) -> dict[str, Any]:
    """Return a JSON-ready representation of one backend result."""

    data = asdict(result)
    data["status"] = result.status.value
    return data
