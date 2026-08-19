"""Send simulator-independent Orion motions to a ROS trajectory controller."""

from __future__ import annotations

import argparse
import math
import sys
from pathlib import Path
from typing import Sequence

import rclpy
from ament_index_python.packages import get_package_share_directory
from builtin_interfaces.msg import Duration
from control_msgs.action import FollowJointTrajectory
from rclpy.action import ActionClient
from rclpy.node import Node
from rclpy.utilities import remove_ros_args
from trajectory_msgs.msg import JointTrajectory, JointTrajectoryPoint

from orion_motion.motion_loader import load_yaml_file
from orion_motion.motion_validator import validate_pose_library
from orion_motion.ros_state_reader import (
    JointStateError,
    MeasuredJointState,
    wait_for_measured_joint_state,
)
from orion_motion.trajectory_builder import (
    ResolvedTrajectory,
    build_trajectory,
)
from orion_motion.trajectory_generator import (
    GeneratedTrajectory,
    TrajectoryGenerationError,
    generate_trajectory,
)


ACTION_NAME = "/joint_trajectory_controller/follow_joint_trajectory"


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


def trajectory_to_message(trajectory: GeneratedTrajectory) -> JointTrajectory:
    """Convert one generated trajectory into a ROS controller message."""

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
) -> GeneratedTrajectory:
    """Generate one requested motion using package limits and measured state."""

    motion_limits = load_yaml_file(
        package_share / "config" / "motion_limits.yaml"
    )
    return generate_trajectory(
        requested,
        start_state.positions,
        start_state.velocities,
        motion_limits,
    )


def duration_seconds(duration: Duration) -> float:
    """Return a ROS duration message as seconds for readable diagnostics."""

    return duration.sec + duration.nanosec / 1_000_000_000


def print_dry_run(
    motion_path: Path,
    trajectory: GeneratedTrajectory,
    message: JointTrajectory,
    *,
    start_pose: str,
) -> None:
    """Print the exact controller goal without contacting an action server."""

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


def send_trajectory_goal(
    node: Node,
    trajectory: JointTrajectory,
    *,
    server_timeout: float,
) -> bool:
    """Send one trajectory goal and wait for its controller result."""

    client = ActionClient(node, FollowJointTrajectory, ACTION_NAME)
    node.get_logger().info(f"Waiting for action server {ACTION_NAME}")
    if not client.wait_for_server(timeout_sec=server_timeout):
        node.get_logger().error(
            f"Action server was unavailable after {server_timeout:.1f} seconds"
        )
        return False

    goal = FollowJointTrajectory.Goal()
    goal.trajectory = trajectory
    send_future = client.send_goal_async(goal)
    rclpy.spin_until_future_complete(node, send_future)
    goal_handle = send_future.result()

    if goal_handle is None or not goal_handle.accepted:
        node.get_logger().error("Trajectory goal was rejected")
        return False

    node.get_logger().info("Trajectory goal accepted")
    result_future = goal_handle.get_result_async()
    rclpy.spin_until_future_complete(node, result_future)
    wrapped_result = result_future.result()
    if wrapped_result is None:
        node.get_logger().error("Trajectory action returned no result")
        return False

    result = wrapped_result.result
    if result.error_code != FollowJointTrajectory.Result.SUCCESSFUL:
        node.get_logger().error(
            f"Trajectory failed with code {result.error_code}: {result.error_string}"
        )
        return False

    node.get_logger().info("Trajectory completed successfully")
    return True


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
        except (TrajectoryGenerationError, ValueError) as error:
            print(f"Cannot generate motion: {error}", file=sys.stderr)
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
        except (JointStateError, TrajectoryGenerationError) as error:
            node.get_logger().error(str(error))
            return 1

        succeeded = send_trajectory_goal(
            node,
            trajectory_to_message(generated),
            server_timeout=options.server_timeout,
        )
        return 0 if succeeded else 1
    finally:
        node.destroy_node()
        rclpy.shutdown()


def main() -> None:
    raise SystemExit(run())
