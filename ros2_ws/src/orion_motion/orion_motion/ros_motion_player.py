"""Send simulator-independent Orion motions to a ROS trajectory controller."""

from __future__ import annotations

import argparse
import math
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Sequence

import rclpy
from ament_index_python.packages import get_package_share_directory
from builtin_interfaces.msg import Duration
from control_msgs.action import FollowJointTrajectory
from control_msgs.msg import JointTolerance
from rclpy.action import ActionClient
from rclpy.node import Node
from rclpy.utilities import remove_ros_args
from trajectory_msgs.msg import JointTrajectory, JointTrajectoryPoint

from orion_motion.motion_loader import load_yaml_file
from orion_motion.motion_validator import (
    MotionValidationError,
    validate_pose_library,
)
from orion_motion.execution_types import (
    ExecutionFeedback,
    ExecutionResult,
    ExecutionStatus,
    JointExecutionState,
)
from orion_motion.ros_state_reader import (
    JointStateError,
    MeasuredJointState,
    require_fresh_measured_state,
    wait_for_measured_joint_state,
)
from orion_motion.trajectory_builder import (
    ResolvedTrajectory,
    build_trajectory,
)
from orion_motion.trajectory_generator import (
    TrajectoryGenerationError,
    generate_trajectory,
)
from orion_motion.trajectory_validator import (
    TrajectoryValidationError,
    ValidatedTrajectory,
    require_valid_trajectory,
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
    result_timeout_factor: float
    result_timeout_margin: float
    cancel_response_timeout: float


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
        "result_timeout_factor",
        "result_timeout_margin",
        "cancel_response_timeout",
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
        result_timeout_factor=float(data["result_timeout_factor"]),
        result_timeout_margin=float(data["result_timeout_margin"]),
        cancel_response_timeout=float(data["cancel_response_timeout"]),
    )


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


def load_named_start_state(
    package_share: Path,
    pose_name: str,
    joint_names: Sequence[str],
) -> MeasuredJointState:
    """Load an explicit stopped start pose for offline dry-run generation."""

    motion_limits = load_yaml_file(
        package_share / "config" / "motion_limits.yaml"
    )
    pose_library = validate_pose_library(
        load_yaml_file(package_share / "config" / "poses.yaml"),
        motion_limits,
    )
    poses = pose_library["poses"]
    if pose_name not in poses:
        available = ", ".join(sorted(poses))
        raise ValueError(
            f"Unknown start pose '{pose_name}'. Available poses: {available}"
        )
    return MeasuredJointState(
        positions=tuple(
            float(poses[pose_name]["positions"][joint_name])
            for joint_name in joint_names
        ),
        velocities=(0.0,) * len(joint_names),
    )


def generate_for_start_state(
    requested: ResolvedTrajectory,
    start_state: MeasuredJointState,
    package_share: Path,
) -> ValidatedTrajectory:
    """Generate and validate one requested motion for execution."""

    motion_limits = load_yaml_file(
        package_share / "config" / "motion_limits.yaml"
    )
    forbidden_regions = load_yaml_file(
        package_share / "config" / "forbidden_regions.yaml"
    )
    generated = generate_trajectory(
        requested,
        start_state.positions,
        start_state.velocities,
        motion_limits,
    )
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
    goal.path_tolerance = [
        JointTolerance(
            name=joint_name,
            position=policy.path_position_tolerance,
        )
        for joint_name in joint_names
    ]
    goal.goal_tolerance = [
        JointTolerance(
            name=joint_name,
            position=policy.goal_position_tolerance,
            velocity=policy.stopped_velocity_tolerance,
        )
        for joint_name in joint_names
    ]
    goal.goal_time_tolerance = seconds_to_duration(
        policy.goal_time_tolerance
    )


_RESULT_STATUSES = {
    FollowJointTrajectory.Result.INVALID_GOAL: ExecutionStatus.INVALID_GOAL,
    FollowJointTrajectory.Result.INVALID_JOINTS: ExecutionStatus.INVALID_JOINTS,
    FollowJointTrajectory.Result.OLD_HEADER_TIMESTAMP: (
        ExecutionStatus.OLD_HEADER_TIMESTAMP
    ),
    FollowJointTrajectory.Result.PATH_TOLERANCE_VIOLATED: (
        ExecutionStatus.PATH_TOLERANCE_VIOLATED
    ),
    FollowJointTrajectory.Result.GOAL_TOLERANCE_VIOLATED: (
        ExecutionStatus.GOAL_TOLERANCE_VIOLATED
    ),
}


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
) -> ExecutionResult:
    """Execute one validated trajectory with bounded waits and full feedback."""

    if not isinstance(validated, ValidatedTrajectory):
        raise TypeError("ROS execution requires a ValidatedTrajectory")
    generated = validated.trajectory
    feedback_samples: list[ExecutionFeedback] = []
    client = action_client or ActionClient(
        node, FollowJointTrajectory, ACTION_NAME
    )
    spin = spin_until_complete or rclpy.spin_until_future_complete
    node.get_logger().info(f"Waiting for action server {ACTION_NAME}")
    if not client.wait_for_server(timeout_sec=server_timeout):
        message = (
            f"Action server was unavailable after {server_timeout:.1f} "
            "seconds"
        )
        node.get_logger().error(message)
        return ExecutionResult(
            motion_name=generated.name,
            backend=BACKEND_NAME,
            status=ExecutionStatus.TIMED_OUT,
            message=message,
        )

    try:
        require_fresh_measured_state(
            start_state,
            policy.max_state_age,
            now=monotonic(),
        )
    except JointStateError as error:
        node.get_logger().error(str(error))
        return ExecutionResult(
            motion_name=generated.name,
            backend=BACKEND_NAME,
            status=ExecutionStatus.REJECTED,
            message=str(error),
        )

    goal = FollowJointTrajectory.Goal()
    goal.trajectory = trajectory_to_message(validated)
    _apply_goal_tolerances(goal, generated.joint_names, policy)

    def receive_feedback(message: Any) -> None:
        feedback_samples.append(feedback_from_message(message.feedback))

    send_future = client.send_goal_async(
        goal,
        feedback_callback=receive_feedback,
    )
    spin(node, send_future, timeout_sec=server_timeout)
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
    goal_handle = send_future.result()

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
    result_future = goal_handle.get_result_async()
    result_timeout = (
        generated.total_duration * policy.result_timeout_factor
        + policy.goal_time_tolerance
        + policy.result_timeout_margin
    )
    spin(node, result_future, timeout_sec=result_timeout)
    if not result_future.done():
        cancel_future = goal_handle.cancel_goal_async()
        spin(
            node,
            cancel_future,
            timeout_sec=policy.cancel_response_timeout,
        )
        message = (
            f"Trajectory result exceeded {result_timeout:.3f}-second "
            "deadline; cancellation requested"
        )
        node.get_logger().error(message)
        return ExecutionResult(
            motion_name=generated.name,
            backend=BACKEND_NAME,
            status=ExecutionStatus.TIMED_OUT,
            message=message,
            feedback=tuple(feedback_samples),
            cancel_requested=True,
        )

    wrapped_result = result_future.result()
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

    result = wrapped_result.result
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

    message = "Trajectory completed successfully"
    node.get_logger().info(message)
    return ExecutionResult(
        motion_name=generated.name,
        backend=BACKEND_NAME,
        status=ExecutionStatus.SUCCEEDED,
        message=message,
        feedback=tuple(feedback_samples),
        backend_error_code=result.error_code,
    )


def positive_float(text: str) -> float:
    """Parse a finite positive command-line number."""

    value = float(text)
    if not math.isfinite(value) or value <= 0:
        raise argparse.ArgumentTypeError("must be a finite number greater than zero")
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
    return parser.parse_args(arguments)


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

    if options.dry_run:
        try:
            start_state = load_named_start_state(
                package_share, options.start_pose, requested.joint_names
            )
            generated = generate_for_start_state(
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

    rclpy.init(args=raw_arguments)
    node = Node("orion_motion_player")
    try:
        try:
            start_state = wait_for_measured_joint_state(
                node,
                requested.joint_names,
                timeout=options.state_timeout,
            )
            generated = generate_for_start_state(
                requested, start_state, package_share
            )
        except (
            JointStateError,
            TrajectoryGenerationError,
            TrajectoryValidationError,
        ) as error:
            node.get_logger().error(str(error))
            return 1

        result = send_trajectory_goal(
            node,
            generated,
            start_state,
            execution_policy,
            server_timeout=options.server_timeout,
        )
        return 0 if result.succeeded else 1
    finally:
        node.destroy_node()
        rclpy.shutdown()


def main() -> None:
    raise SystemExit(run())
