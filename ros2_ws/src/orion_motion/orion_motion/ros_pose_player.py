"""Send a named Orion pose to a ROS trajectory controller."""

from __future__ import annotations

import argparse
import math
import sys
from pathlib import Path
from typing import Sequence

import rclpy
from ament_index_python.packages import get_package_share_directory
from rclpy.node import Node
from rclpy.utilities import remove_ros_args

from orion_motion.motion_loader import load_yaml_file
from orion_motion.ros_motion_player import (
    ACTION_NAME,
    duration_seconds,
    positive_float,
    send_trajectory_goal,
    trajectory_to_message,
)
from orion_motion.trajectory_builder import (
    ResolvedTrajectory,
    build_pose_trajectory,
)


def nonnegative_float(text: str) -> float:
    """Parse a finite, non-negative command-line number."""

    value = float(text)
    if not math.isfinite(value) or value < 0:
        raise argparse.ArgumentTypeError(
            "must be a finite number greater than or equal to zero"
        )
    return value


def load_installed_pose_trajectory(
    pose_name: str,
    duration: float,
    hold: float,
    *,
    package_share: Path | None = None,
) -> tuple[Path, ResolvedTrajectory]:
    """Load package configuration and resolve one requested named pose."""

    share = package_share or Path(get_package_share_directory("orion_motion"))
    pose_path = share / "config" / "poses.yaml"
    pose_library = load_yaml_file(pose_path)
    motion_limits = load_yaml_file(share / "config" / "motion_limits.yaml")
    trajectory = build_pose_trajectory(
        pose_name,
        duration,
        pose_library,
        motion_limits,
        hold=hold,
    )
    return pose_path, trajectory


def print_dry_run(
    pose_path: Path,
    pose_name: str,
    trajectory: ResolvedTrajectory,
) -> None:
    """Print the exact named-pose controller goal without contacting ROS."""

    message = trajectory_to_message(trajectory)
    print(f"Pose: {pose_name}")
    print(f"Source: {pose_path}")
    print(f"Action: {ACTION_NAME}")
    print(f"Joints: {', '.join(message.joint_names)}")
    print("Trajectory points:")
    for index, point in enumerate(message.points):
        positions = ", ".join(f"{value:+.3f}" for value in point.positions)
        print(
            f"  {index}: t={duration_seconds(point.time_from_start):.3f} s "
            f"positions=[{positions}]"
        )


def parse_arguments(arguments: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Move Orion directly to one installed named pose."
    )
    parser.add_argument("pose", help="Pose name, such as home or attentive.")
    parser.add_argument(
        "--duration",
        type=positive_float,
        default=1.5,
        help="Travel time in seconds (default: 1.5).",
    )
    parser.add_argument(
        "--hold",
        type=nonnegative_float,
        default=0.0,
        help="Time to remain at the pose before completing (default: 0).",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the controller goal without contacting ROS action servers.",
    )
    parser.add_argument(
        "--server-timeout",
        type=positive_float,
        default=10.0,
        help="Seconds to wait for the trajectory action server (default: 10).",
    )
    return parser.parse_args(arguments)


def run(arguments: Sequence[str] | None = None) -> int:
    """Run the named-pose CLI and return a process exit code."""

    raw_arguments = list(arguments) if arguments is not None else sys.argv
    cli_arguments = remove_ros_args(args=raw_arguments)[1:]
    options = parse_arguments(cli_arguments)
    pose_path, trajectory = load_installed_pose_trajectory(
        options.pose,
        options.duration,
        options.hold,
    )

    if options.dry_run:
        print_dry_run(pose_path, options.pose, trajectory)
        return 0

    rclpy.init(args=raw_arguments)
    node = Node("orion_pose_player")
    try:
        succeeded = send_trajectory_goal(
            node,
            trajectory_to_message(trajectory),
            server_timeout=options.server_timeout,
        )
        return 0 if succeeded else 1
    finally:
        node.destroy_node()
        rclpy.shutdown()


def main() -> None:
    raise SystemExit(run())
