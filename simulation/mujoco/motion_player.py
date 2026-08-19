"""Play simulator-independent Orion motion files in MuJoCo."""

from __future__ import annotations

import argparse
import math
import sys
import time
from pathlib import Path
from typing import Any

import mujoco
import mujoco.viewer

from mujoco_backend import (
    MuJoCoJointMapping,
    read_joint_positions,
    resolve_joint_mapping,
    set_actuator_targets,
    set_joint_state,
)


PROJECT_ROOT = Path(__file__).resolve().parents[2]
MOTION_PACKAGE_SOURCE = PROJECT_ROOT / "ros2_ws" / "src" / "orion_motion"
CONFIG_DIRECTORY = MOTION_PACKAGE_SOURCE / "config"
MOTIONS_DIRECTORY = MOTION_PACKAGE_SOURCE / "motions"
DEFAULT_SCENE = Path(__file__).resolve().parent / "scene.xml"

# This simulator adapter consumes the package directly from the source tree.
# The orion_motion package itself remains independent of MuJoCo.
sys.path.insert(0, str(MOTION_PACKAGE_SOURCE))

from orion_motion.motion_loader import load_yaml_file  # noqa: E402
from orion_motion.trajectory_builder import build_trajectory  # noqa: E402
from orion_motion.trajectory_generator import (  # noqa: E402
    GeneratedTrajectory,
    generate_trajectory,
    sample_trajectory,
)


def find_motion_file(motion_name: str) -> Path:
    """Find one packaged motion by filename and reject ambiguous names."""

    matches = sorted(
        path for path in MOTIONS_DIRECTORY.rglob("*.yaml") if path.stem == motion_name
    )
    if not matches:
        available = ", ".join(
            sorted(path.stem for path in MOTIONS_DIRECTORY.rglob("*.yaml"))
        )
        raise ValueError(
            f"Unknown motion '{motion_name}'. Available motions: {available}"
        )
    if len(matches) > 1:
        locations = ", ".join(str(path) for path in matches)
        raise ValueError(f"Motion name '{motion_name}' is ambiguous: {locations}")
    return matches[0]


def load_playback_data(
    motion_name: str, start_pose_name: str
) -> tuple[Path, GeneratedTrajectory, tuple[float, ...]]:
    """Load and generate a motion from an explicit stopped simulation pose."""

    motion_path = find_motion_file(motion_name)
    motion_definition = load_yaml_file(motion_path)
    pose_library = load_yaml_file(CONFIG_DIRECTORY / "poses.yaml")
    motion_limits = load_yaml_file(CONFIG_DIRECTORY / "motion_limits.yaml")
    requested = build_trajectory(
        motion_definition,
        pose_library,
        motion_limits,
    )

    if requested.name != motion_name:
        raise ValueError(
            f"Motion file '{motion_path.name}' declares name '{requested.name}'"
        )

    poses = pose_library["poses"]
    if start_pose_name not in poses:
        available = ", ".join(sorted(poses))
        raise ValueError(
            f"Unknown start pose '{start_pose_name}'. Available poses: {available}"
        )

    start_positions = tuple(
        float(poses[start_pose_name]["positions"][joint_name])
        for joint_name in requested.joint_names
    )
    generated = generate_trajectory(
        requested,
        start_positions,
        (0.0,) * len(requested.joint_names),
        motion_limits,
    )
    return motion_path, generated, start_positions


def run_playback_loop(
    model: mujoco.MjModel,
    data: mujoco.MjData,
    mapping: MuJoCoJointMapping,
    trajectory: GeneratedTrajectory,
    *,
    lead_in: float,
    viewer: Any | None,
) -> bool:
    """Execute one trajectory; return false if its viewer closes early."""

    playback_start = data.time + lead_in
    completed = False
    completion_reported = False

    while viewer is None or viewer.is_running():
        step_started = time.perf_counter()

        if data.time >= playback_start:
            elapsed = data.time - playback_start
            desired, completed = sample_trajectory(trajectory, elapsed)
            set_actuator_targets(data, mapping, desired.positions)

        mujoco.mj_step(model, data)

        if viewer is not None:
            viewer.sync()

        if completed and not completion_reported:
            print(f"Playback complete at simulation time {data.time:.3f} s")
            completion_reported = True
            if viewer is None:
                break

        if viewer is not None:
            remaining = model.opt.timestep - (time.perf_counter() - step_started)
            if remaining > 0:
                time.sleep(remaining)

    return completed


def report_final_error(
    data: mujoco.MjData,
    mapping: MuJoCoJointMapping,
    trajectory: GeneratedTrajectory,
) -> float:
    """Print final measured positions and return the largest absolute error."""

    desired = trajectory.points[-1].positions
    measured = read_joint_positions(data, mapping)
    errors = tuple(
        actual - target
        for actual, target in zip(measured, desired, strict=True)
    )

    print("Final joint results:")
    for name, target, actual, error in zip(
        mapping.joint_names, desired, measured, errors, strict=True
    ):
        print(
            f"  {name}: target={target:+.6f} "
            f"measured={actual:+.6f} error={error:+.6f}"
        )

    maximum_error = max(abs(error) for error in errors)
    print(f"Maximum absolute final error: {maximum_error:.6f} rad")
    return maximum_error


def play_motion(
    scene_path: Path,
    trajectory: GeneratedTrajectory,
    start_positions: tuple[float, ...],
    *,
    lead_in: float,
    headless: bool,
) -> bool:
    """Initialize MuJoCo and play one shared generated Orion trajectory."""

    model = mujoco.MjModel.from_xml_path(str(scene_path))
    data = mujoco.MjData(model)
    mapping = resolve_joint_mapping(model, trajectory.joint_names)
    set_joint_state(model, data, mapping, start_positions)

    if headless:
        completed = run_playback_loop(
            model,
            data,
            mapping,
            trajectory,
            lead_in=lead_in,
            viewer=None,
        )
    else:
        with mujoco.viewer.launch_passive(model, data) as viewer:
            viewer.cam.lookat[:] = [0.02, 0.0, 0.20]
            viewer.cam.distance = 0.75
            viewer.cam.azimuth = 90
            viewer.cam.elevation = -10
            completed = run_playback_loop(
                model,
                data,
                mapping,
                trajectory,
                lead_in=lead_in,
                viewer=viewer,
            )

    report_final_error(data, mapping, trajectory)
    return completed


def nonnegative_float(text: str) -> float:
    """Parse a finite, non-negative command-line number."""

    value = float(text)
    if not math.isfinite(value) or value < 0:
        raise argparse.ArgumentTypeError("must be a finite number greater than or equal to 0")
    return value


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Play a simulator-independent Orion motion in MuJoCo."
    )
    parser.add_argument("motion", help="Packaged motion name, such as look_at_left.")
    parser.add_argument(
        "--start-pose",
        default="attentive",
        help="Named pose used to initialize the fresh simulation (default: attentive).",
    )
    parser.add_argument(
        "--scene",
        type=Path,
        default=DEFAULT_SCENE,
        help="MuJoCo scene XML path.",
    )
    parser.add_argument(
        "--lead-in",
        type=nonnegative_float,
        default=1.0,
        help="Simulation seconds to show the start pose before playback (default: 1.0).",
    )
    parser.add_argument(
        "--headless",
        action="store_true",
        help="Run to completion without opening a viewer.",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_arguments()
    motion_path, trajectory, start_positions = load_playback_data(
        args.motion, args.start_pose
    )

    print(f"Motion: {trajectory.name}")
    print(f"Source: {motion_path}")
    print(f"Start pose: {args.start_pose}")
    print(f"Motion duration: {trajectory.total_duration:.3f} s")

    completed = play_motion(
        args.scene.resolve(),
        trajectory,
        start_positions,
        lead_in=args.lead_in,
        headless=args.headless,
    )
    if not completed:
        raise SystemExit("Playback stopped before the motion completed")


if __name__ == "__main__":
    main()
