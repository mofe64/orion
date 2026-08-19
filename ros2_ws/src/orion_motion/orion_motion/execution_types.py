"""Backend-neutral records for Orion motion execution."""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum


class ExecutionStatus(str, Enum):
    """Terminal outcome preserved across execution backends."""

    SUCCEEDED = "succeeded"
    REJECTED = "rejected"
    INVALID_GOAL = "invalid_goal"
    INVALID_JOINTS = "invalid_joints"
    OLD_HEADER_TIMESTAMP = "old_header_timestamp"
    PATH_TOLERANCE_VIOLATED = "path_tolerance_violated"
    GOAL_TOLERANCE_VIOLATED = "goal_tolerance_violated"
    TIMED_OUT = "timed_out"
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
class ExecutionResult:
    """Terminal execution outcome with all feedback retained for analysis."""

    motion_name: str
    backend: str
    status: ExecutionStatus
    message: str
    feedback: tuple[ExecutionFeedback, ...] = ()
    backend_error_code: int | None = None
    cancel_requested: bool = False

    @property
    def succeeded(self) -> bool:
        """Return whether execution reached the backend's success state."""

        return self.status is ExecutionStatus.SUCCEEDED
