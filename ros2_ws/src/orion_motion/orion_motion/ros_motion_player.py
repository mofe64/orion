"""Send simulator-independent Orion motions to a ROS trajectory controller."""

from __future__ import annotations

import argparse
from dataclasses import dataclass, replace
import math
from pathlib import Path
import sys
import threading
import time
from typing import Any, Callable, Sequence

from action_msgs.msg import GoalStatus
from ament_index_python.packages import get_package_share_directory
from builtin_interfaces.msg import Duration
from control_msgs.action import FollowJointTrajectory
from control_msgs.msg import JointTolerance
import rclpy
from rclpy.action import ActionClient
from rclpy.node import Node
from rclpy.signals import SignalHandlerOptions
from rclpy.utilities import remove_ros_args
from trajectory_msgs.msg import JointTrajectory, JointTrajectoryPoint

from orion_motion.execution_types import (
    ExecutionFeedback,
    ExecutionMetrics,
    ExecutionResult,
    ExecutionStatus,
    JointExecutionState,
    execution_metrics_from_feedback,
    execution_result_data,
)
from orion_motion.motion_loader import load_yaml_file
from orion_motion.motion_validator import (
    MotionValidationError,
    validate_pose_library,
)
from orion_motion.ros_state_reader import (
    JointStateError,
    MeasuredJointState,
    require_fresh_measured_state,
    wait_for_measured_joint_state,
    wait_for_settled_joint_state,
    wait_for_stopped_joint_state,
)
from orion_motion.reporting import build_run_report, write_json_report
from orion_motion.ros_stability_monitor import (
    RosBaseStabilityMonitor,
    ros_base_stability_policy_from_data,
)
from orion_motion.trajectory_builder import (
    build_trajectory,
    ResolvedTrajectory,
)
from orion_motion.trajectory_generator import (
    generate_trajectory,
    TrajectoryGenerationError,
)
from orion_motion.trajectory_validator import (
    require_valid_trajectory,
    TrajectoryValidationError,
    ValidatedTrajectory,
)


ACTION_NAME = "/joint_trajectory_controller/follow_joint_trajectory"
BACKEND_NAME = "ros2_control"


@dataclass(frozen=True)
class RosExecutionPolicy:
    """Versioned bounds for one ROS trajectory-controller interaction."""

    max_state_age: float
    path_position_tolerance: float
    goal_position_tolerance: float
    stopped_velocity_tolerance: float
    goal_time_tolerance: float
    goal_settle_duration: float
    goal_settle_timeout: float
    result_timeout_factor: float
    result_timeout_margin: float
    cancel_response_timeout: float
    stop_confirmation_timeout: float
    stop_confirmation_duration: float


def execution_policy_from_data(data: Any) -> RosExecutionPolicy:
    """Validate and build a typed ROS execution policy."""

    if not isinstance(data, dict):
        raise MotionValidationError("execution_policy must be a mapping")
    if type(data.get("format_version")) is not int or data["format_version"] != 1:
        raise MotionValidationError(
            "execution_policy.format_version must be the integer 1"
        )
    if data.get("applicability") != "provisional_simulation_only":
        raise MotionValidationError(
            "execution_policy.applicability must be "
            "'provisional_simulation_only'"
        )
    fields = (
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
    )
    for field_name in fields:
        value = data.get(field_name)
        if (
            isinstance(value, bool)
            or not isinstance(value, (int, float))
            or not math.isfinite(value)
            or value <= 0
        ):
            raise MotionValidationError(
                f"execution_policy.{field_name} must be a finite number "
                "greater than zero"
            )

    return RosExecutionPolicy(
        max_state_age=float(data["max_state_age"]),
        path_position_tolerance=float(
            data["path_position_tolerance"]
        ),
        goal_position_tolerance=float(
            data["goal_position_tolerance"]
        ),
        stopped_velocity_tolerance=float(
            data["stopped_velocity_tolerance"]
        ),
        goal_time_tolerance=float(data["goal_time_tolerance"]),
        goal_settle_duration=float(data["goal_settle_duration"]),
        goal_settle_timeout=float(data["goal_settle_timeout"]),
        result_timeout_factor=float(data["result_timeout_factor"]),
        result_timeout_margin=float(data["result_timeout_margin"]),
        cancel_response_timeout=float(data["cancel_response_timeout"]),
        stop_confirmation_timeout=float(data["stop_confirmation_timeout"]),
        stop_confirmation_duration=float(data["stop_confirmation_duration"]),
    )


class GoalCancellation:
    """Own one idempotent cancellation request for one accepted ROS goal."""

    # Orion may stop a motion because the user cancelled it, a newer motion
    # replaced it, or the controller took too long to finish it.
    _ALLOWED_REASONS = {
        ExecutionStatus.CANCELLED,
        ExecutionStatus.PREEMPTED,
        ExecutionStatus.TIMED_OUT,
    }

    def __init__(self) -> None:
        """Create a cancellation state before or after goal acceptance."""

        # Orion creates one GoalCancellation for each motion it starts. The motion
        # queue and the ROS execution code share this same object.

        # Protect the data below because the motion queue and ROS callbacks may
        # read or change it at the same time.
        self._lock = threading.Lock()

        # This is Orion's reference to the goal accepted by the trajectory
        # controller. It is the exact goal we will ask the controller to cancel.
        self._goal_handle: Any | None = None

        # This future will later contain the controller's reply to our cancel
        # request. A reply does not by itself prove that Orion has stopped moving.
        self._cancel_future: Any | None = None

        # Remember whether Orion is stopping because of a direct cancellation,
        # a replacement motion, or a timeout.
        self._reason: ExecutionStatus | None = None

        # Remember the newest real joint positions reported before cancellation.
        self._latest_positions: tuple[float, ...] | None = None

        # Freeze the joint positions from the moment cancellation first starts.
        # Orion later compares these with the final stopped positions.
        self._requested_positions: tuple[float, ...] | None = None

        # Remember when cancellation started so Orion can measure stopping time.
        self._requested_at: float | None = None

    @property
    def reason(self) -> ExecutionStatus | None:
        """Return why the goal is being cancelled, if requested."""

        # Before sending a goal, the motion queue reads this to see whether the
        # request was already cancelled or replaced. After a cancelled result,
        # Orion reads it again to report why the motion stopped.
        with self._lock:
            # Use the lock so the reason cannot change while Orion reads it.
            return self._reason

    @property
    def cancel_future(self) -> Any | None:
        """Return the one controller cancellation future, when available."""

        # This lets Orion read the same controller reply without sending another
        # cancel request. It is None until the goal exists and the request is sent.
        with self._lock:
            return self._cancel_future

    @property
    def request_snapshot(self) -> tuple[float, tuple[float, ...]] | None:
        """Return the request time and measured joint positions captured when
        cancellation began."""

        # After Orion confirms that the joints have stopped, the cancellation
        # metrics code reads this to calculate stopping time and joint movement.
        with self._lock:
            # A complete snapshot needs both the request time and joint positions.
            if self._requested_at is None or self._requested_positions is None:
                return None

            # Orion uses these values to measure stopping time and distance.
            return self._requested_at, self._requested_positions

    def observe_positions(self, positions: Sequence[float]) -> None:
        """Remember the newest measured positions until cancellation begins."""

        # Orion calls this first with the starting joint positions, then again
        # whenever the controller sends new feedback while the robot is moving.

        # These are the actual measured positions, not the commanded targets.
        # Copy them into a fixed tuple so we keep one clear snapshot.
        measured = tuple(float(position) for position in positions)

        # Invalid joint feedback cannot be used to measure Orion's stopping motion.
        if any(not math.isfinite(position) for position in measured):
            raise ValueError("observed positions must be finite")

        with self._lock:
            # Keep the newest measurement until the first cancel request arrives.
            # After that, stop updating it so Orion can measure how far each joint
            # moved between the cancel request and the confirmed stop.
            if self._requested_at is None:
                self._latest_positions = measured

    def attach(self, goal_handle: Any) -> None:
        """Attach the accepted goal and honour any earlier cancel request."""
        # this method attaches the goal, so that we have a reference to the goal
        # in case we want to cancel it

        # send_trajectory_goal() calls this after the trajectory controller accepts
        # Orion's FollowJointTrajectory goal. Before this point, there is no
        # accepted controller goal that Orion can ask to cancel.
        with self._lock:
            # Save the handle for the exact controller goal Orion is running.
            self._goal_handle = goal_handle

            # A cancel or replacement may have arrived while Orion was waiting for
            # goal acceptance. If so, send that saved request now.
            # if there is no cancellation/replacement request will be a no-op
            self._request_once_locked()

    def request(self, reason: ExecutionStatus) -> Any | None:
        """Request cancellation once and return the shared cancel future."""

        # The motion queue calls this for a direct cancel or replacement motion.
        # The execution code also calls it when the user interrupts a run or the
        # controller does not return a result before Orion's time limit.

        # Reject statuses such as success or failure because they are results,
        # not reasons for asking the controller to cancel an active goal.
        if reason not in self._ALLOWED_REASONS:
            raise ValueError(f"invalid cancellation reason: {reason.value}")

        with self._lock:
            # Keep the first stopping reason. A direct cancellation may replace a
            # previous replacement or timeout reason because it is more explicit.
            if self._reason is None or reason is ExecutionStatus.CANCELLED:
                self._reason = reason

            # Save one starting point for the whole stopping measurement. Later
            # calls must not reset the time or positions while Orion is stopping.
            if self._requested_at is None:
                self._requested_at = time.monotonic()
                self._requested_positions = self._latest_positions

            # Send now if the controller has accepted the goal. Otherwise, keep
            # the reason and snapshot until attach() receives the goal handle.
            self._request_once_locked()

            # Every caller receives the same future. It is None only when Orion is
            # still waiting for the controller to accept the goal.
            return self._cancel_future

    def _request_once_locked(self) -> None:
        # request() and attach() call this while already holding the lock. Keeping
        # this check inside the lock prevents them from sending two cancel requests
        # for the same controller goal at the same time.

        # Send only when cancellation was requested, the controller goal exists,
        # and Orion has not already sent a cancel request for it.
        if (
            self._reason is not None
            and self._goal_handle is not None
            and self._cancel_future is None
        ):
            # Ask the trajectory controller to cancel the goal. This returns at
            # once; the future receives the controller's reply later. Orion still
            # checks the goal result and measured joint feedback before it says the
            # robot has stopped.
            self._cancel_future = self._goal_handle.cancel_goal_async()


class LatestMotionRequestQueue:
    """Keep one newest pending request and preempt an active older request."""

    def __init__(self) -> None:
        """Create an empty one-slot request queue."""

        # The ROS execution loop and feedback callbacks can use this queue at the
        # same time. The lock stops them from changing the queue together.
        self._lock = threading.Lock()

        # Store the newest motion waiting to start. There is only one waiting
        # place, so a newer request replaces an older request that has not started.
        self._pending: ResolvedTrajectory | None = None

        # Store the cancellation control for the motion currently running. This
        # lets a new request ask the old motion to stop before the new one starts.
        self._active_cancellation: GoalCancellation | None = None

    def submit(self, requested: ResolvedTrajectory) -> None:
        """Replace the pending request and interrupt any active request."""

        # Orion calls this for the first requested motion and for every replacement
        # motion that arrives while another motion may still be running.
        with self._lock:
            # Keep only the newest motion. If two replacements arrive quickly, the
            # second one replaces the first before the execution loop takes it.
            self._pending = requested

            # Remember the running motion's cancellation control, if one exists.
            active = self._active_cancellation

        # Do this after releasing the queue lock. GoalCancellation has its own lock
        # and may need to send a request to the trajectory controller.
        if active is not None:
            # PREEMPTED means the old motion is stopping because a newer one won.
            active.request(ExecutionStatus.PREEMPTED)

    def cancel(self) -> None:
        """Discard pending work and cancel the active request, if any."""

        # Orion calls this when the user wants motion to stop without starting a
        # replacement. It must remove both waiting work and current work.
        with self._lock:
            # Remove the waiting motion so it cannot start after the current one.
            self._pending = None

            # Remember the running motion's cancellation control, if one exists.
            active = self._active_cancellation

        # Release the queue lock before asking GoalCancellation to stop the goal.
        if active is not None:
            # CANCELLED records that this was a direct stop, not a replacement.
            active.request(ExecutionStatus.CANCELLED)

    def take_latest(self) -> ResolvedTrajectory | None:
        """Remove and return the newest pending request."""

        # execute_motion_queue() calls this when it is ready to start another
        # motion. None tells the loop that there is no more work to run.
        with self._lock:
            # Take ownership of the waiting motion and empty the waiting place.
            # A replacement submitted after this point becomes the next motion.
            requested = self._pending
            self._pending = None
            return requested

    def set_active(self, cancellation: GoalCancellation) -> None:
        """Register cancellation control for the request being executed."""

        # After taking a motion from the queue, execute_motion_queue() creates a
        # GoalCancellation for it and registers that object here.
        with self._lock:
            # New submit() or cancel() calls can now stop this running motion.
            self._active_cancellation = cancellation

            # A replacement may have arrived after take_latest() but before this
            # method. Remember that race so the older motion does not continue.
            newer_request_waiting = self._pending is not None

        # If a newer motion is already waiting, stop this older motion and let the
        # execution loop move on to the newer one.
        if newer_request_waiting:
            cancellation.request(ExecutionStatus.PREEMPTED)

    def clear_active(self, cancellation: GoalCancellation) -> None:
        """Clear the active request if it still matches the caller."""

        # execute_motion_queue() calls this after a motion finishes, fails, or is
        # cancelled, so later requests do not try to stop an old finished goal.
        with self._lock:
            # Clear only the same cancellation object that finished. This prevents
            # old cleanup code from clearing a newer active motion by mistake.
            if self._active_cancellation is cancellation:
                self._active_cancellation = None


def load_execution_policy(package_share: Path) -> RosExecutionPolicy:
    """Load the installed execution policy used by the ROS action adapter."""

    return execution_policy_from_data(
        load_yaml_file(package_share / "config" / "execution_policy.yaml")
    )


def seconds_to_duration(seconds: float) -> Duration:
    """Convert finite, non-negative seconds to an exact ROS duration message."""

    if not math.isfinite(seconds) or seconds < 0:
        raise ValueError("ROS trajectory time must be finite and non-negative")

    whole_seconds = math.floor(seconds)
    nanoseconds = round((seconds - whole_seconds) * 1_000_000_000)
    if nanoseconds == 1_000_000_000:
        whole_seconds += 1
        nanoseconds = 0
    return Duration(sec=whole_seconds, nanosec=nanoseconds)


def trajectory_to_message(validated: ValidatedTrajectory) -> JointTrajectory:
    """Convert one validated trajectory into a ROS controller message."""

    if not isinstance(validated, ValidatedTrajectory):
        raise TypeError("ROS execution requires a ValidatedTrajectory")
    trajectory = validated.trajectory

    message = JointTrajectory()
    message.joint_names = list(trajectory.joint_names)

    for generated_point in trajectory.points:
        point = JointTrajectoryPoint()
        point.positions = list(generated_point.positions)
        point.velocities = list(generated_point.velocities)
        point.accelerations = list(generated_point.accelerations)
        point.time_from_start = seconds_to_duration(
            generated_point.time_from_start
        )
        message.points.append(point)

    return message


def find_motion_file(package_share: Path, motion_name: str) -> Path:
    """Find one installed motion by filename and reject ambiguous names."""

    motions_directory = package_share / "motions"
    matches = sorted(
        path for path in motions_directory.rglob("*.yaml") if path.stem == motion_name
    )
    if not matches:
        available = ", ".join(
            sorted(path.stem for path in motions_directory.rglob("*.yaml"))
        )
        raise ValueError(
            f"Unknown motion '{motion_name}'. Available motions: {available}"
        )
    if len(matches) > 1:
        locations = ", ".join(str(path) for path in matches)
        raise ValueError(f"Motion name '{motion_name}' is ambiguous: {locations}")
    return matches[0]


def load_installed_trajectory(
    motion_name: str,
    *,
    package_share: Path | None = None,
) -> tuple[Path, ResolvedTrajectory]:
    """Load and resolve one motion from an installed Orion package."""

    share = package_share or Path(get_package_share_directory("orion_motion"))
    motion_path = find_motion_file(share, motion_name)
    motion_definition = load_yaml_file(motion_path)
    pose_library = load_yaml_file(share / "config" / "poses.yaml")
    motion_limits = load_yaml_file(share / "config" / "motion_limits.yaml")
    trajectory = build_trajectory(
        motion_definition,
        pose_library,
        motion_limits,
    )

    if trajectory.name != motion_name:
        raise ValueError(
            f"Motion file '{motion_path.name}' declares name '{trajectory.name}'"
        )
    return motion_path, trajectory


def build_dry_run_start_state_from_pose(
    package_share: Path,
    pose_name: str,
    joint_names: Sequence[str],
) -> MeasuredJointState:
    """Build an assumed stopped start state from a pose for a dry run."""

    # Orion calls this only for --dry-run, when there is no live joint feedback.
    # Real execution reads the starting positions and velocities from the robot.

    # Load the motion limits so the pose library can be checked before it is used.
    motion_limits = load_yaml_file(
        package_share / "config" / "motion_limits.yaml"
    )

    # Load the named poses and check their joints, units, and position limits.
    pose_library = validate_pose_library(
        load_yaml_file(package_share / "config" / "poses.yaml"),
        motion_limits,
    )
    poses = pose_library["poses"]

    # Give the user a clear list if the requested dry-run start pose does not exist.
    if pose_name not in poses:
        available = ", ".join(sorted(poses))
        raise ValueError(
            f"Unknown start pose '{pose_name}'. Available poses: {available}"
        )

    # Build the positions in the same joint order as the requested trajectory.
    # Set every velocity to zero because a dry run assumes Orion starts stopped.
    return MeasuredJointState(
        positions=tuple(
            float(poses[pose_name]["positions"][joint_name])
            for joint_name in joint_names
        ),
        velocities=(0.0,) * len(joint_names),
    )


def generate_validated_trajectory_from_start_state(
    requested: ResolvedTrajectory,
    start_state: MeasuredJointState,
    package_share: Path,
) -> ValidatedTrajectory:
    """Generate and validate a trajectory from Orion's given start state."""

    # Real execution passes a state measured from Orion. A dry run passes the
    # assumed stopped state built from a named pose by the function above.

    # Use the same motion limits for both trajectory generation and validation.
    motion_limits = load_yaml_file(
        package_share / "config" / "motion_limits.yaml"
    )

    # Forbidden regions describe joint combinations Orion must not move through.
    forbidden_regions = load_yaml_file(
        package_share / "config" / "forbidden_regions.yaml"
    )

    # Turn the resolved pose targets into timed trajectory points that begin at
    # the supplied starting positions and velocities.
    generated = generate_trajectory(
        requested,
        start_state.positions,
        start_state.velocities,
        motion_limits,
    )

    # Check all generated points and segments before they can be printed in a dry
    # run or sent to the real trajectory controller.
    return require_valid_trajectory(
        generated, motion_limits, forbidden_regions
    )


def duration_seconds(duration: Duration) -> float:
    """Return a ROS duration message as seconds for readable diagnostics."""

    return duration.sec + duration.nanosec / 1_000_000_000


def print_dry_run(
    motion_path: Path,
    validated: ValidatedTrajectory,
    message: JointTrajectory,
    *,
    start_pose: str,
) -> None:
    """Print the exact controller goal without contacting an action server."""

    trajectory = validated.trajectory
    print(f"Motion: {trajectory.name}")
    print(f"Source: {motion_path}")
    print(f"Dry-run start pose: {start_pose}")
    print(f"Action: {ACTION_NAME}")
    print(f"Joints: {', '.join(message.joint_names)}")
    print("Trajectory points:")
    for index, point in enumerate(message.points):
        positions = ", ".join(f"{value:+.3f}" for value in point.positions)
        velocities = ", ".join(f"{value:+.3f}" for value in point.velocities)
        accelerations = ", ".join(
            f"{value:+.3f}" for value in point.accelerations
        )
        print(
            f"  {index}: t={duration_seconds(point.time_from_start):.3f} s "
            f"positions=[{positions}] velocities=[{velocities}] "
            f"accelerations=[{accelerations}]"
        )


def _execution_state_from_message(point: JointTrajectoryPoint) -> JointExecutionState:
    return JointExecutionState(
        positions=tuple(point.positions),
        velocities=tuple(point.velocities),
        accelerations=tuple(point.accelerations),
        time_from_start=duration_seconds(point.time_from_start),
    )


def feedback_from_message(feedback: Any) -> ExecutionFeedback:
    """Convert ROS action feedback without losing desired/actual/error state."""

    return ExecutionFeedback(
        timestamp=(
            feedback.header.stamp.sec
            + feedback.header.stamp.nanosec / 1_000_000_000
        ),
        joint_names=tuple(feedback.joint_names),
        desired=_execution_state_from_message(feedback.desired),
        actual=_execution_state_from_message(feedback.actual),
        error=_execution_state_from_message(feedback.error),
    )


def _apply_goal_tolerances(
    goal: FollowJointTrajectory.Goal,
    joint_names: Sequence[str],
    policy: RosExecutionPolicy,
) -> None:
    """Tell the trajectory controller how closely Orion must follow the goal."""

    # send_trajectory_goal() calls this after adding the validated trajectory and
    # before sending the FollowJointTrajectory goal to the controller.

    # These values come from execution_policy.yaml. They do not change the motion
    # points. They tell the controller when tracking error is too large.

    # While Orion is moving, each real joint position must stay this close to its
    # desired position. If it moves farther away, the controller stops the goal
    # and reports a path-tolerance failure.
    goal.path_tolerance = [
        JointTolerance(
            name=joint_name,
            position=policy.path_position_tolerance,
        )
        for joint_name in joint_names
    ]

    # At the planned end, every joint must be close to its final position and
    # moving slowly enough to count as stopped. If not, the controller reports a
    # goal-tolerance failure.
    goal.goal_tolerance = [
        JointTolerance(
            name=joint_name,
            position=policy.goal_position_tolerance,
            velocity=policy.stopped_velocity_tolerance,
        )
        for joint_name in joint_names
    ]

    # Give the controller this much extra time after the planned end to enter the
    # final position and velocity limits before it marks the goal as failed.
    goal.goal_time_tolerance = seconds_to_duration(
        policy.goal_time_tolerance
    ) # A successful controller result is not Orion's final physical check. Orion later reads fresh joint feedback and confirms that the final pose stays still.


# The trajectory controller returns ROS error codes when a goal fails. Orion uses
# this table to turn each known ROS code into its own ExecutionStatus for logs,
# reports, tests, and callers of the motion player.
# Successful results are handled by the normal success path, so they do not need
# an entry here. Any unknown error code becomes the general FAILED status later.
_RESULT_STATUSES = {
    # The controller rejected the structure or timing of the trajectory goal.
    FollowJointTrajectory.Result.INVALID_GOAL: ExecutionStatus.INVALID_GOAL,

    # The goal's joint names do not match the joints managed by the controller.
    FollowJointTrajectory.Result.INVALID_JOINTS: ExecutionStatus.INVALID_JOINTS,

    # The goal asked the controller to start from a timestamp that was already old.
    FollowJointTrajectory.Result.OLD_HEADER_TIMESTAMP: (
        ExecutionStatus.OLD_HEADER_TIMESTAMP
    ),

    # A real joint moved too far from its desired position during the motion.
    FollowJointTrajectory.Result.PATH_TOLERANCE_VIOLATED: (
        ExecutionStatus.PATH_TOLERANCE_VIOLATED
    ),

    # A joint did not reach the final limits within the allowed finishing time.
    FollowJointTrajectory.Result.GOAL_TOLERANCE_VIOLATED: (
        ExecutionStatus.GOAL_TOLERANCE_VIOLATED
    ),
}




def _confirm_stopped_after_cancel(
    node: Node,
    joint_names: Sequence[str],
    policy: RosExecutionPolicy,
    *,
    stopped_state_waiter: Callable[..., MeasuredJointState],
) -> tuple[bool, str, MeasuredJointState | None]:
    try:
        stopped_state = stopped_state_waiter(
            node,
            joint_names,
            maximum_velocity=policy.stopped_velocity_tolerance,
            stable_duration=policy.stop_confirmation_duration,
            timeout=policy.stop_confirmation_timeout,
        )
    except (JointStateError, ValueError) as error:
        return False, f"stop could not be confirmed: {error}", None
    return True, "fresh joint feedback confirmed a stopped state", stopped_state


def _cancellation_metrics(
    feedback: Sequence[ExecutionFeedback],
    cancellation: GoalCancellation,
    stopped_state: MeasuredJointState | None,
) -> ExecutionMetrics:
    """Add measured stopping time and distance to ordinary feedback metrics."""

    metrics = execution_metrics_from_feedback(feedback)
    snapshot = cancellation.request_snapshot
    if snapshot is None or stopped_state is None:
        return metrics
    requested_at, requested_positions = snapshot
    if len(requested_positions) != len(stopped_state.positions):
        return metrics
    return replace(
        metrics,
        final_velocities=stopped_state.velocities,
        cancellation_stopping_time=max(0.0, time.monotonic() - requested_at),
        cancellation_stopping_distances=tuple(
            abs(stopped - requested)
            for requested, stopped in zip(
                requested_positions,
                stopped_state.positions,
                strict=True,
            )
        ),
    )


def _finish_requested_cancellation(
    node: Node,
    result_future: Any,
    cancellation: GoalCancellation,
    reason: ExecutionStatus,
    joint_names: Sequence[str],
    policy: RosExecutionPolicy,
    *,
    spin: Callable[..., Any],
    stopped_state_waiter: Callable[..., MeasuredJointState],
) -> tuple[bool, str, MeasuredJointState | None]:
    """Wait for one cancellation request, its result, and a measured stop."""

    # send_trajectory_goal() calls this when the user interrupts an active motion
    # or when the controller takes too long to return the action result.

    # This helper checks three separate things:
    # 1. Did the controller answer Orion's cancel request?
    # 2. Did the action goal return a final result?
    # 3. Does fresh joint feedback show that Orion has physically stopped?

    # Ask GoalCancellation to send its one cancel request for this goal. It also
    # remembers the reason, request time, and measured positions at this moment.
    cancel_future = cancellation.request(reason)

    # Collect each outcome so the final ExecutionResult can explain what happened.
    details: list[str] = []

    # This is a defensive case for cancellation before an accepted goal handle is
    # available. Without a goal handle, Orion cannot contact a specific ROS goal.
    if cancel_future is None:
        details.append("goal was not available for cancellation")
    else:
        # Keep processing ROS messages until the controller answers or Orion's
        # configured cancellation-response time limit expires.
        spin(
            node,
            cancel_future,
            timeout_sec=policy.cancel_response_timeout,
        )

        # A future that is still unfinished means no reply arrived in time.
        if not cancel_future.done():
            details.append("controller cancellation response timed out")
        else:
            # The cancel future contains the controller's response to the request.
            response = cancel_future.result()
            goals_canceling = getattr(response, "goals_canceling", None)

            # ROS normally lists the goals it accepted for cancellation. An empty
            # list means the controller did not accept this goal for cancellation.
            if goals_canceling is not None and not goals_canceling:
                details.append("controller did not accept the cancellation")
            else:
                details.append("controller accepted the cancellation")

    # The cancel response only says whether the request was accepted. Orion must
    # separately wait for the original trajectory action to reach a final state.
    if not result_future.done():
        # Give the controller a short, bounded time to finish the cancelled goal.
        spin(
            node,
            result_future,
            timeout_sec=policy.cancel_response_timeout,
        )

    # Record whether the original action supplied a final result in time.
    if not result_future.done():
        details.append("cancelled goal did not return a terminal result")
    else:
        # A ROS action result wraps both the controller result and action status.
        wrapped_result = result_future.result()
        wrapped_status = getattr(wrapped_result, "status", None)

        # STATUS_CANCELED confirms the action ended through ROS cancellation. A
        # different status can occur if the goal finished or failed at the same
        # time that Orion requested cancellation.
        if wrapped_status == GoalStatus.STATUS_CANCELED:
            details.append("controller reported the goal as cancelled")
        else:
            details.append("controller returned after the cancellation request")

    # Controller replies describe software state, not physical movement. Always
    # read fresh joint feedback and check that every Orion joint stays slow enough
    # for long enough to count as stopped.
    stopped, stop_detail, stopped_state = _confirm_stopped_after_cancel(
        node,
        joint_names,
        policy,
        stopped_state_waiter=stopped_state_waiter,
    )
    details.append(stop_detail)

    # Return whether stopping was confirmed, one readable account of all three
    # checks, and the final measured state used for stopping-distance metrics.
    return stopped, "; ".join(details), stopped_state


def send_trajectory_goal(
    node: Node,
    validated: ValidatedTrajectory,
    start_state: MeasuredJointState,
    policy: RosExecutionPolicy,
    *,
    server_timeout: float,
    action_client: Any | None = None,
    spin_until_complete: Callable[..., Any] | None = None,
    monotonic: Callable[[], float] = time.monotonic,
    cancellation: GoalCancellation | None = None,
    stopped_state_waiter: Callable[..., MeasuredJointState] | None = None,
    settled_state_waiter: Callable[..., MeasuredJointState] | None = None,
    feedback_observer: Callable[[ExecutionFeedback], None] | None = None,
) -> ExecutionResult:
    """Execute one validated trajectory with bounded waits and full feedback."""

    # execute_motion_queue() calls this after it has read Orion's joint state,
    # generated a trajectory from that state, and passed trajectory validation.
    # This method owns the ROS action conversation with the trajectory controller.

    # Do not let normal ROS execution bypass Orion's trajectory validator. The
    # ValidatedTrajectory type is the proof that the generated path passed checks.
    if not isinstance(validated, ValidatedTrajectory):
        raise TypeError("ROS execution requires a ValidatedTrajectory")

    # Keep a shorter name for the generated trajectory stored inside the validated
    # wrapper. It contains Orion's joint order, points, timing, and motion name.
    generated = validated.trajectory

    # Save every controller feedback sample so the final result can report desired
    # positions, measured positions, errors, and maximum tracking error.
    feedback_samples: list[ExecutionFeedback] = []

    # Normal execution creates a client for Orion's FollowJointTrajectory action.
    # Tests may pass a fake client so they can exercise this flow without ROS.
    client = action_client or ActionClient(
        node, FollowJointTrajectory, ACTION_NAME
    )

    # Spinning lets ROS process replies and feedback while Orion waits for a future.
    # Tests replace it with a small controlled function.
    spin = spin_until_complete or rclpy.spin_until_future_complete

    # The motion queue supplies this object so a user cancel or replacement can
    # reach the active goal. A direct call gets its own cancellation state.
    cancellation_state = cancellation or GoalCancellation()

    # These waiters read real joint feedback in normal execution. Tests can replace
    # them with known measured states for cancellation and settling checks.
    wait_for_stopped = stopped_state_waiter or wait_for_stopped_joint_state
    wait_for_settled = settled_state_waiter or wait_for_settled_joint_state

    # Use the measured starting positions as the first cancellation snapshot. New
    # controller feedback will replace them until cancellation actually begins.
    cancellation_state.observe_positions(start_state.positions)

    # First check that the trajectory controller's action server can be reached.
    # This wait is bounded so Orion cannot hang forever when control is unavailable.
    node.get_logger().info(f"Waiting for action server {ACTION_NAME}")
    if not client.wait_for_server(timeout_sec=server_timeout):
        message = (
            f"Action server was unavailable after {server_timeout:.1f} "
            "seconds"
        )
        node.get_logger().error(message)

        # No goal was sent, so there is nothing to cancel or physically stop here.
        return ExecutionResult(
            motion_name=generated.name,
            backend=BACKEND_NAME,
            status=ExecutionStatus.TIMED_OUT,
            message=message,
        )

    # The trajectory was generated from start_state. Check its age again after
    # waiting for the server, because Orion may have moved while time passed.
    # Sending a path from an old position could cause a jump at the first point.
    try:
        require_fresh_measured_state(
            start_state,
            policy.max_state_age,
            now=monotonic(),
        )
    except JointStateError as error:
        node.get_logger().error(str(error))

        # Reject before contacting the controller. The caller can read a fresh
        # state, regenerate the trajectory, and try again safely.
        return ExecutionResult(
            motion_name=generated.name,
            backend=BACKEND_NAME,
            status=ExecutionStatus.REJECTED,
            message=str(error),
        )

    # Build the ROS action goal from Orion's validated points. The tolerance helper
    # tells the controller how much error is allowed during and after the motion.
    goal = FollowJointTrajectory.Goal()
    goal.trajectory = trajectory_to_message(validated)
    _apply_goal_tolerances(goal, generated.joint_names, policy)

    def receive_feedback(message: Any) -> None:
        # The controller calls this repeatedly while the goal is running. Convert
        # the ROS message into Orion's backend-neutral feedback record.
        sample = feedback_from_message(message.feedback)
        feedback_samples.append(sample)

        # Cancellation needs the newest actual measured positions so Orion can
        # later calculate how far each joint moved while stopping.
        cancellation_state.observe_positions(sample.actual.positions)

        # The outer execution flow may also observe feedback to trigger a timed
        # cancellation, submit a replacement, or record extra execution evidence.
        if feedback_observer is not None:
            feedback_observer(sample)

    # Send the goal without blocking. send_future will later contain the controller
    # goal handle, which tells Orion whether this specific goal was accepted.
    send_future = client.send_goal_async(
        goal,
        feedback_callback=receive_feedback,
    )

    # Process ROS messages while waiting, but only up to the server time limit.
    spin(node, send_future, timeout_sec=server_timeout)

    # An unfinished send future means Orion did not receive an acceptance or
    # rejection reply in time, so it cannot continue with this goal safely.
    if not send_future.done():
        message = "Timed out waiting for trajectory goal response"
        node.get_logger().error(message)
        return ExecutionResult(
            motion_name=generated.name,
            backend=BACKEND_NAME,
            status=ExecutionStatus.TIMED_OUT,
            message=message,
            feedback=tuple(feedback_samples),
        )

    # The accepted goal handle is Orion's reference to this controller goal. It is
    # needed to wait for the result or ask the controller to cancel this goal.
    goal_handle = send_future.result()

    # A completed send future can still say that the controller rejected the goal.
    # Rejection means the trajectory never became an active controller command.
    if goal_handle is None or not goal_handle.accepted:
        message = "Trajectory goal was rejected"
        node.get_logger().error(message)
        return ExecutionResult(
            motion_name=generated.name,
            backend=BACKEND_NAME,
            status=ExecutionStatus.REJECTED,
            message=message,
            feedback=tuple(feedback_samples),
        )

    node.get_logger().info("Trajectory goal accepted")

    # Connect GoalCancellation to this accepted goal. If a cancel or replacement
    # arrived while Orion waited for acceptance, attach() sends that request now.
    cancellation_state.attach(goal_handle)

    # result_future is different from send_future. It completes when the accepted
    # action ends, not when the controller first accepts the goal.
    result_future = goal_handle.get_result_async()

    # Give the controller enough wall-clock time for the planned motion, its final
    # tolerance window, and a communication margin. The factor allows simulation
    # to run slower than real time without allowing an endless wait.
    result_timeout = (
        generated.total_duration * policy.result_timeout_factor
        + policy.goal_time_tolerance
        + policy.result_timeout_margin
    )
    try:
        # While this spins, ROS can deliver motion feedback, cancellation requests,
        # and the final action result.
        spin(node, result_future, timeout_sec=result_timeout)
    except KeyboardInterrupt:
        # A keyboard interrupt is a user cancellation. Ask the controller to stop,
        # wait for its final result, and confirm the joints physically stopped.
        stopped, detail, stopped_state = _finish_requested_cancellation(
            node,
            result_future,
            cancellation_state,
            ExecutionStatus.CANCELLED,
            generated.joint_names,
            policy,
            spin=spin,
            stopped_state_waiter=wait_for_stopped,
        )
        message = f"Motion cancelled by user; {detail}"
        node.get_logger().info(message)

        # Preserve feedback and stopping measurements even though the motion did
        # not finish normally.
        return ExecutionResult(
            motion_name=generated.name,
            backend=BACKEND_NAME,
            status=ExecutionStatus.CANCELLED,
            message=message,
            feedback=tuple(feedback_samples),
            cancel_requested=True,
            stop_confirmed=stopped,
            metrics=_cancellation_metrics(
                feedback_samples, cancellation_state, stopped_state
            ),
        )

    # If the result future is unfinished after the deadline, the controller did
    # not finish the goal in time. Cancel it instead of leaving motion active.
    if not result_future.done():
        stopped, detail, stopped_state = _finish_requested_cancellation(
            node,
            result_future,
            cancellation_state,
            ExecutionStatus.TIMED_OUT,
            generated.joint_names,
            policy,
            spin=spin,
            stopped_state_waiter=wait_for_stopped,
        )
        message = (
            f"Trajectory result exceeded {result_timeout:.3f}-second "
            f"deadline; {detail}"
        )
        node.get_logger().error(message)

        # The status remains TIMED_OUT even if cancellation and stopping succeed;
        # those fields separately record whether the timeout was handled safely.
        return ExecutionResult(
            motion_name=generated.name,
            backend=BACKEND_NAME,
            status=ExecutionStatus.TIMED_OUT,
            message=message,
            feedback=tuple(feedback_samples),
            cancel_requested=True,
            stop_confirmed=stopped,
            metrics=_cancellation_metrics(
                feedback_samples, cancellation_state, stopped_state
            ),
        )

    # The result future returns a ROS wrapper containing both the action status and
    # the FollowJointTrajectory controller result.
    wrapped_result = result_future.result()

    # A completed future should contain a wrapper. Treat a missing one as a general
    # execution failure rather than claiming success.
    if wrapped_result is None:
        message = "Trajectory action returned no result"
        node.get_logger().error(message)
        return ExecutionResult(
            motion_name=generated.name,
            backend=BACKEND_NAME,
            status=ExecutionStatus.FAILED,
            message=message,
            feedback=tuple(feedback_samples),
        )

    # result contains the controller error code and message. action_status says how
    # ROS ended the action, such as succeeded, cancelled, or aborted.
    result = wrapped_result.result
    action_status = getattr(
        wrapped_result,
        "status",
        GoalStatus.STATUS_SUCCEEDED,
    )

    # Cancellation may have come from the user, a newer replacement motion, or a
    # timeout path. Use GoalCancellation to preserve that Orion-level reason.
    if action_status == GoalStatus.STATUS_CANCELED:
        reason = cancellation_state.reason or ExecutionStatus.CANCELLED

        # The action status says the software goal ended. Fresh joint feedback must
        # still prove that the physical or simulated joints have stopped.
        stopped, detail, stopped_state = _confirm_stopped_after_cancel(
            node,
            generated.joint_names,
            policy,
            stopped_state_waiter=wait_for_stopped,
        )
        message = f"Trajectory {reason.value}; {detail}"
        node.get_logger().info(message)

        # Return the correct Orion reason together with stop confirmation and the
        # distance each joint travelled after cancellation was requested.
        return ExecutionResult(
            motion_name=generated.name,
            backend=BACKEND_NAME,
            status=reason,
            message=message,
            feedback=tuple(feedback_samples),
            backend_error_code=result.error_code,
            cancel_requested=cancellation_state.reason is not None,
            stop_confirmed=stopped,
            metrics=_cancellation_metrics(
                feedback_samples, cancellation_state, stopped_state
            ),
        )

    # A non-success controller code gives a more useful reason than a general ROS
    # aborted status. Translate known FollowJointTrajectory codes into Orion names.
    if result.error_code != FollowJointTrajectory.Result.SUCCESSFUL:
        status = _RESULT_STATUSES.get(
            result.error_code, ExecutionStatus.FAILED
        )
        message = (
            f"Trajectory failed with code {result.error_code}: "
            f"{result.error_string}"
        )
        node.get_logger().error(message)
        return ExecutionResult(
            motion_name=generated.name,
            backend=BACKEND_NAME,
            status=status,
            message=message,
            feedback=tuple(feedback_samples),
            backend_error_code=result.error_code,
        )

    # Require both layers to agree on success: the controller error code above and
    # the outer ROS action status here. A mismatch is treated as a failure.
    if action_status != GoalStatus.STATUS_SUCCEEDED:
        message = f"Trajectory action ended with status {action_status}"
        node.get_logger().error(message)
        return ExecutionResult(
            motion_name=generated.name,
            backend=BACKEND_NAME,
            status=ExecutionStatus.FAILED,
            message=message,
            feedback=tuple(feedback_samples),
            backend_error_code=result.error_code,
        )

    # Controller success is not enough to prove Orion reached a stable final pose.
    # Start a separate measured settling check using fresh joint-state messages.
    settle_started = monotonic()
    try:
        # Every joint must stay close to the final target and below the stopped
        # velocity limit for the full settle duration, within a bounded timeout.
        settled_state = wait_for_settled(
            node,
            generated.joint_names,
            generated.points[-1].positions,
            maximum_position_error=policy.goal_position_tolerance,
            maximum_velocity=policy.stopped_velocity_tolerance,
            stable_duration=policy.goal_settle_duration,
            timeout=policy.goal_settle_timeout,
        )
    except (JointStateError, ValueError) as error:
        # The controller completed its action, but measured feedback did not prove
        # a stable final pose. Keep this distinct from an action/controller error.
        message = f"Trajectory ended but did not settle: {error}"
        node.get_logger().error(message)
        return ExecutionResult(
            motion_name=generated.name,
            backend=BACKEND_NAME,
            status=ExecutionStatus.SETTLING_FAILED,
            message=message,
            feedback=tuple(feedback_samples),
            backend_error_code=result.error_code,
            metrics=execution_metrics_from_feedback(feedback_samples),
        )

    # Summarize tracking error from action feedback, then add final measurements
    # from the independent settling check. ExecutionMetrics is frozen, so replace()
    # creates a new record instead of changing the original one.
    base_metrics = execution_metrics_from_feedback(feedback_samples)
    final_target = generated.points[-1].positions
    metrics = replace(
        base_metrics,
        final_position_errors=tuple(
            target - actual
            for target, actual in zip(
                final_target,
                settled_state.positions,
                strict=True,
            )
        ),
        final_velocities=settled_state.velocities,
        settling_time=max(0.0, monotonic() - settle_started),
    )

    # Only this path means the controller succeeded and fresh joint feedback proved
    # that Orion reached the target and remained still.
    message = "Trajectory completed and remained settled"
    node.get_logger().info(message)
    return ExecutionResult(
        motion_name=generated.name,
        backend=BACKEND_NAME,
        status=ExecutionStatus.SUCCEEDED,
        message=message,
        feedback=tuple(feedback_samples),
        backend_error_code=result.error_code,
        metrics=metrics,
    )


def execute_motion_queue(
    node: Node,
    requests: LatestMotionRequestQueue,
    package_share: Path,
    policy: RosExecutionPolicy,
    *,
    state_timeout: float,
    server_timeout: float,
    state_reader: Callable[..., MeasuredJointState] = (
        wait_for_measured_joint_state
    ),
    goal_sender: Callable[..., ExecutionResult] = send_trajectory_goal,
    action_client: Any | None = None,
    execution_observer: Callable[
        [ValidatedTrajectory, MeasuredJointState, float, ExecutionResult],
        None,
    ]
    | None = None,
    execution_feedback_observer: Callable[
        [ResolvedTrajectory, ExecutionFeedback], None
    ]
    | None = None,
    execution_started: Callable[[ValidatedTrajectory], None] | None = None,
    result_transformer: Callable[[ExecutionResult], ExecutionResult]
    | None = None,
) -> tuple[ExecutionResult, ...]:
    """Execute requests serially, keeping only the newest replacement."""

    results: list[ExecutionResult] = []
    active_action_client = (
        action_client
        if action_client is not None
        else (
            ActionClient(node, FollowJointTrajectory, ACTION_NAME)
            if goal_sender is send_trajectory_goal
            else None
        )
    )
    while True:
        requested = requests.take_latest()
        if requested is None:
            break

        cancellation = GoalCancellation()
        requests.set_active(cancellation)

        def skip_interrupted_request() -> bool:
            reason = cancellation.reason
            if reason is None:
                return False
            results.append(
                ExecutionResult(
                    motion_name=requested.name,
                    backend=BACKEND_NAME,
                    status=reason,
                    message=(
                        f"Request {reason.value} before a movement goal "
                        "was sent"
                    ),
                )
            )
            return True

        try:
            if skip_interrupted_request():
                continue

            if (
                active_action_client is not None
                and not active_action_client.wait_for_server(
                    timeout_sec=server_timeout
                )
            ):
                message = (
                    f"Action server was unavailable after "
                    f"{server_timeout:.1f} seconds"
                )
                node.get_logger().error(message)
                results.append(
                    ExecutionResult(
                        motion_name=requested.name,
                        backend=BACKEND_NAME,
                        status=ExecutionStatus.TIMED_OUT,
                        message=message,
                    )
                )
                continue

            if skip_interrupted_request():
                continue

            try:
                start_state = state_reader(
                    node,
                    requested.joint_names,
                    timeout=state_timeout,
                )
                start_state_age = start_state.age()
                validated = generate_validated_trajectory_from_start_state(
                    requested,
                    start_state,
                    package_share,
                )
            except (
                JointStateError,
                TrajectoryGenerationError,
                TrajectoryValidationError,
            ) as error:
                node.get_logger().error(str(error))
                results.append(
                    ExecutionResult(
                        motion_name=requested.name,
                        backend=BACKEND_NAME,
                        status=ExecutionStatus.REJECTED,
                        message=str(error),
                    )
                )
                continue

            if skip_interrupted_request():
                continue

            if execution_started is not None:
                execution_started(validated)
            sender_arguments = {
                "server_timeout": server_timeout,
                "cancellation": cancellation,
            }
            if active_action_client is not None:
                sender_arguments["action_client"] = active_action_client
            if execution_feedback_observer is not None:
                sender_arguments["feedback_observer"] = lambda sample: (
                    execution_feedback_observer(requested, sample)
                )
            result = goal_sender(
                node,
                validated,
                start_state,
                policy,
                **sender_arguments,
            )
            if result_transformer is not None:
                result = result_transformer(result)
            if execution_observer is not None:
                execution_observer(
                    validated,
                    start_state,
                    start_state_age,
                    result,
                )
        finally:
            requests.clear_active(cancellation)
        results.append(result)

        if result.status is ExecutionStatus.CANCELLED:
            requests.cancel()

    return tuple(results)


def positive_float(text: str) -> float:
    """Parse a finite positive command-line number."""

    value = float(text)
    if not math.isfinite(value) or value <= 0:
        raise argparse.ArgumentTypeError("must be a finite number greater than zero")
    return value


def nonempty_text(text: str) -> str:
    """Reject empty labels used to identify an execution backend."""

    value = text.strip()
    if not value:
        raise argparse.ArgumentTypeError("must be a non-empty string")
    return value


def parse_arguments(arguments: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Play an installed Orion motion through ROS 2 control."
    )
    parser.add_argument("motion", help="Motion name, such as look_at_left.")
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the controller goal without contacting ROS action servers.",
    )
    parser.add_argument(
        "--start-pose",
        default="attentive",
        help=(
            "Stopped named start used only for --dry-run generation "
            "(default: attentive)."
        ),
    )
    parser.add_argument(
        "--state-timeout",
        type=positive_float,
        default=3.0,
        help="Seconds to wait for measured joint state (default: 3).",
    )
    parser.add_argument(
        "--server-timeout",
        type=positive_float,
        default=10.0,
        help="Seconds to wait for the trajectory action server (default: 10).",
    )
    parser.add_argument(
        "--report-json",
        type=Path,
        help="Optional path for a complete machine-readable run report.",
    )
    parser.add_argument(
        "--backend-label",
        type=nonempty_text,
        default=BACKEND_NAME,
        help=(
            "Backend recorded in --report-json, such as gazebo or "
            "mujoco_ros2_control (default: ros2_control)."
        ),
    )
    lifecycle = parser.add_mutually_exclusive_group()
    lifecycle.add_argument(
        "--cancel-at",
        type=positive_float,
        metavar="SECONDS",
        help=(
            "Cancel from controller feedback when trajectory time reaches "
            "this value."
        ),
    )
    lifecycle.add_argument(
        "--replace-with",
        metavar="MOTION",
        help="Replace the active motion with this named motion.",
    )
    parser.add_argument(
        "--replace-at",
        type=positive_float,
        metavar="SECONDS",
        help=(
            "Controller trajectory time at which --replace-with is submitted."
        ),
    )
    options = parser.parse_args(arguments)
    if (options.replace_with is None) != (options.replace_at is None):
        parser.error("--replace-with and --replace-at must be used together")
    if options.dry_run and (
        options.cancel_at is not None or options.replace_with is not None
    ):
        parser.error("lifecycle triggers cannot be used with --dry-run")
    return options


def run(arguments: Sequence[str] | None = None) -> int:
    """Run the CLI and return a process exit code."""

    raw_arguments = list(arguments) if arguments is not None else sys.argv
    cli_arguments = remove_ros_args(args=raw_arguments)[1:]
    options = parse_arguments(cli_arguments)

    package_share = Path(get_package_share_directory("orion_motion"))
    execution_policy = load_execution_policy(package_share)
    motion_path, requested = load_installed_trajectory(
        options.motion, package_share=package_share
    )
    replacement_path: Path | None = None
    replacement: ResolvedTrajectory | None = None
    trigger_time = options.cancel_at
    if options.replace_with is not None:
        replacement_path, replacement = load_installed_trajectory(
            options.replace_with,
            package_share=package_share,
        )
        trigger_time = options.replace_at
    if trigger_time is not None and trigger_time >= requested.total_duration:
        print(
            f"Lifecycle trigger {trigger_time:.3f} s must be earlier than "
            f"the initial motion duration {requested.total_duration:.3f} s",
            file=sys.stderr,
        )
        return 2

    if options.dry_run:
        try:
            start_state = build_dry_run_start_state_from_pose(
                package_share, options.start_pose, requested.joint_names
            )
            generated = generate_validated_trajectory_from_start_state(
                requested, start_state, package_share
            )
        except (
            TrajectoryGenerationError,
            TrajectoryValidationError,
            ValueError,
        ) as error:
            print(f"Cannot prepare motion: {error}", file=sys.stderr)
            return 1
        message = trajectory_to_message(generated)
        print_dry_run(
            motion_path,
            generated,
            message,
            start_pose=options.start_pose,
        )
        return 0

    rclpy.init(
        args=raw_arguments,
        signal_handler_options=SignalHandlerOptions.NO,
    )
    node = Node("orion_motion_player")
    stability_monitor: RosBaseStabilityMonitor | None = None
    try:
        observed: list[
            tuple[
                ValidatedTrajectory,
                MeasuredJointState,
                float,
                ExecutionResult,
            ]
        ] = []

        def observe_execution(
            validated: ValidatedTrajectory,
            start_state: MeasuredJointState,
            start_state_age: float,
            result: ExecutionResult,
        ) -> None:
            observed.append(
                (validated, start_state, start_state_age, result)
            )

        requests = LatestMotionRequestQueue()
        requests.submit(requested)
        if options.backend_label in ("gazebo", "gazebo_ros2_control"):
            stability_monitor = RosBaseStabilityMonitor(
                node,
                ros_base_stability_policy_from_data(
                    load_yaml_file(
                        package_share / "config" / "stability_limits.yaml"
                    )
                ),
            )
        lifecycle_triggered = False

        def observe_feedback(
            active_request: ResolvedTrajectory,
            sample: ExecutionFeedback,
        ) -> None:
            nonlocal lifecycle_triggered
            if lifecycle_triggered or trigger_time is None:
                return
            if active_request is not requested:
                return
            if sample.actual.time_from_start < trigger_time:
                return
            lifecycle_triggered = True
            if replacement is None:
                node.get_logger().info(
                    f"Requesting cancellation at trajectory time "
                    f"{sample.actual.time_from_start:.3f} s"
                )
                requests.cancel()
            else:
                node.get_logger().info(
                    f"Replacing {requested.name} with {replacement.name} at "
                    f"trajectory time {sample.actual.time_from_start:.3f} s"
                )
                requests.submit(replacement)

        results = execute_motion_queue(
            node,
            requests,
            package_share,
            execution_policy,
            state_timeout=options.state_timeout,
            server_timeout=options.server_timeout,
            execution_observer=observe_execution,
            execution_feedback_observer=observe_feedback,
            execution_started=(
                (lambda unused: stability_monitor.begin())
                if stability_monitor is not None
                else None
            ),
            result_transformer=(
                stability_monitor.enrich_result
                if stability_monitor is not None
                else None
            ),
        )
        if not results:
            return 1
        result = results[-1]
        if options.report_json is not None:
            if not observed:
                node.get_logger().error(
                    "Run report unavailable because no trajectory was executed"
                )
                return 1
            validated, start_state, start_state_age, observed_result = observed[-1]
            labeled_result = replace(
                observed_result,
                backend=options.backend_label,
            )
            report_motion_path = (
                replacement_path
                if replacement is not None
                and validated.trajectory.name == replacement.name
                else motion_path
            )
            report = build_run_report(
                motion_path=report_motion_path,
                limits_path=package_share / "config" / "motion_limits.yaml",
                validated=validated,
                start_positions=start_state.positions,
                start_velocities=start_state.velocities,
                start_state_age=start_state_age,
                result=labeled_result,
            )
            if trigger_time is not None:
                report["lifecycle"] = {
                    "triggered": lifecycle_triggered,
                    "trigger_time": trigger_time,
                    "operation": (
                        "cancel" if replacement is None else "replace"
                    ),
                    "replacement_motion": (
                        replacement.name if replacement is not None else None
                    ),
                    "results": [
                        execution_result_data(
                            replace(item, backend=options.backend_label)
                        )
                        for item in results
                    ],
                }
            write_json_report(options.report_json, report)
            node.get_logger().info(
                f"Machine-readable report: {options.report_json.resolve()}"
            )
        if options.cancel_at is not None:
            return (
                0
                if lifecycle_triggered
                and len(results) == 1
                and result.status is ExecutionStatus.CANCELLED
                and result.stop_confirmed
                else 1
            )
        if replacement is not None:
            return (
                0
                if lifecycle_triggered
                and len(results) == 2
                and results[0].status is ExecutionStatus.PREEMPTED
                and results[0].stop_confirmed
                and results[1].succeeded
                else 1
            )
        return 0 if result.succeeded else 1
    finally:
        if stability_monitor is not None:
            stability_monitor.close()
        node.destroy_node()
        if rclpy.ok():
            rclpy.shutdown()


def main() -> None:
    raise SystemExit(run())
