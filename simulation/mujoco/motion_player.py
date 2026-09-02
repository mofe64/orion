"""Play simulator-independent Orion motion files in MuJoCo."""

from __future__ import annotations

import argparse
import json
import math
import sys
import time
from pathlib import Path
from typing import Any

import mujoco
import mujoco.viewer

from mujoco_backend import (
    MuJoCoJointMapping,
    read_joint_accelerations,
    read_joint_positions,
    read_joint_velocities,
    resolve_joint_mapping,
    set_actuator_targets,
    set_joint_state,
)
from stability_monitor import (
    StabilityMonitor,
    StabilityPolicy,
    StabilitySnapshot,
    stability_policy_from_data,
)


PROJECT_ROOT = Path(__file__).resolve().parents[2]
MOTION_SOURCE = PROJECT_ROOT / "motion"
CONFIG_DIRECTORY = MOTION_SOURCE / "config"
MOTIONS_DIRECTORY = MOTION_SOURCE / "motions"
DEFAULT_SCENE = Path(__file__).resolve().parent / "scene.xml"

# This simulator adapter consumes the backend-independent motion library
# directly from the source tree.
sys.path.insert(0, str(MOTION_SOURCE))

from orion_motion.motion_loader import load_yaml_file  # noqa: E402
from orion_motion.execution_types import (  # noqa: E402
    ExecutionFeedback,
    ExecutionMetrics,
    ExecutionResult,
    ExecutionStatus,
    JointExecutionState,
    execution_result_data,
)
from orion_motion.compiled_trajectory import (  # noqa: E402
    CompiledTrajectory,
    TrajectoryCompilerError,
    compile_trajectory,
    sample_trajectory,
)


BACKEND_NAME = "native_mujoco"


def load_stability_policy() -> StabilityPolicy:
    """Load the shared native-simulation completion and stability policy."""

    return stability_policy_from_data(
        load_yaml_file(CONFIG_DIRECTORY / "stability_limits.yaml")
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
) -> tuple[Path, CompiledTrajectory, tuple[float, ...]]:
    """Load one Rust-compiled v2 motion for MuJoCo execution."""

    motion_path = find_motion_file(motion_name)
    compiled = compile_trajectory(
        motion_name,
        start_pose_name,
        pose_file=CONFIG_DIRECTORY / "poses.yaml",
        motions_directory=MOTIONS_DIRECTORY,
    )
    return motion_path, compiled, compiled.samples[0].positions


def run_playback_loop(
    model: mujoco.MjModel,
    data: mujoco.MjData,
    mapping: MuJoCoJointMapping,
    trajectory: CompiledTrajectory,
    *,
    lead_in: float,
    viewer: Any | None,
    policy: StabilityPolicy | None = None,
) -> ExecutionResult:
    """Execute one trajectory and require measured settling and stability."""

    if not isinstance(trajectory, CompiledTrajectory):
        raise TypeError("MuJoCo execution requires a Rust CompiledTrajectory")
    active_policy = policy or load_stability_policy()
    playback_start = data.time + lead_in
    feedback_samples: list[ExecutionFeedback] = []
    maximum_position_errors = [0.0] * len(mapping.joint_names)
    monitor: StabilityMonitor | None = None
    latest_stability: StabilitySnapshot | None = None
    settled_since: float | None = None

    def metrics(settling_time: float | None) -> ExecutionMetrics:
        actual_positions = read_joint_positions(data, mapping)
        actual_velocities = read_joint_velocities(data, mapping)
        final_positions = trajectory.points[-1].positions
        final_errors = tuple(
            desired - actual
            for desired, actual in zip(
                final_positions,
                actual_positions,
                strict=True,
            )
        )
        return ExecutionMetrics(
            maximum_position_errors=tuple(maximum_position_errors),
            final_position_errors=final_errors,
            final_velocities=actual_velocities,
            settling_time=settling_time,
            maximum_base_translation=(
                latest_stability.maximum_translation
                if latest_stability is not None
                else None
            ),
            maximum_base_tilt=(
                latest_stability.maximum_tilt
                if latest_stability is not None
                else None
            ),
            maximum_base_height_change=(
                latest_stability.maximum_height_change
                if latest_stability is not None
                else None
            ),
            longest_contact_loss=(
                latest_stability.longest_contact_loss
                if latest_stability is not None
                else None
            ),
        )

    while True:
        if viewer is not None and not viewer.is_running():
            return ExecutionResult(
                motion_name=trajectory.name,
                backend=BACKEND_NAME,
                status=ExecutionStatus.CANCELLED,
                message="MuJoCo viewer closed before a terminal result",
                feedback=tuple(feedback_samples),
                metrics=metrics(None),
            )

        step_started = time.perf_counter()

        if data.time < playback_start:
            mujoco.mj_step(model, data)
            if viewer is not None:
                viewer.sync()
            continue

        if monitor is None:
            monitor = StabilityMonitor(model, data, active_policy)

        elapsed = data.time - playback_start
        desired, _ = sample_trajectory(trajectory, elapsed)
        set_actuator_targets(data, mapping, desired.positions)

        mujoco.mj_step(model, data)

        actual_positions = read_joint_positions(data, mapping)
        actual_velocities = read_joint_velocities(data, mapping)
        actual_accelerations = read_joint_accelerations(data, mapping)
        position_errors = tuple(
            target - actual
            for target, actual in zip(
                desired.positions,
                actual_positions,
                strict=True,
            )
        )
        velocity_errors = tuple(
            target - actual
            for target, actual in zip(
                desired.velocities,
                actual_velocities,
                strict=True,
            )
        )
        acceleration_errors = tuple(
            target - actual
            for target, actual in zip(
                desired.accelerations,
                actual_accelerations,
                strict=True,
            )
        )
        for index, error in enumerate(position_errors):
            maximum_position_errors[index] = max(
                maximum_position_errors[index],
                abs(error),
            )

        feedback_samples.append(
            ExecutionFeedback(
                timestamp=float(data.time),
                joint_names=tuple(mapping.joint_names),
                desired=JointExecutionState(
                    positions=tuple(desired.positions),
                    velocities=tuple(desired.velocities),
                    accelerations=tuple(desired.accelerations),
                    time_from_start=float(desired.time_from_start),
                ),
                actual=JointExecutionState(
                    positions=actual_positions,
                    velocities=actual_velocities,
                    accelerations=actual_accelerations,
                    time_from_start=max(0.0, float(data.time - playback_start)),
                ),
                error=JointExecutionState(
                    positions=position_errors,
                    velocities=velocity_errors,
                    accelerations=acceleration_errors,
                    time_from_start=max(0.0, float(data.time - playback_start)),
                ),
            )
        )
        latest_stability = monitor.update()

        if viewer is not None:
            viewer.sync()

        if not latest_stability.safe:
            message = "; ".join(latest_stability.unsafe_reasons)
            return ExecutionResult(
                motion_name=trajectory.name,
                backend=BACKEND_NAME,
                status=ExecutionStatus.UNSAFE_STABILITY,
                message=message,
                feedback=tuple(feedback_samples),
                metrics=metrics(None),
            )

        elapsed_after_step = max(0.0, float(data.time - playback_start))
        if elapsed_after_step >= trajectory.total_duration:
            final_positions = trajectory.points[-1].positions
            position_ok = all(
                abs(target - actual) <= active_policy.position_tolerance
                for target, actual in zip(
                    final_positions,
                    actual_positions,
                    strict=True,
                )
            )
            velocity_ok = all(
                abs(value) <= active_policy.velocity_tolerance
                for value in actual_velocities
            )
            if position_ok and velocity_ok:
                if settled_since is None:
                    settled_since = float(data.time)
                settled_for = float(data.time) - settled_since
                if settled_for >= active_policy.settle_duration:
                    settling_time = max(
                        0.0,
                        float(data.time - playback_start)
                        - trajectory.total_duration,
                    )
                    return ExecutionResult(
                        motion_name=trajectory.name,
                        backend=BACKEND_NAME,
                        status=ExecutionStatus.SUCCEEDED,
                        message=(
                            "Trajectory reached the final pose and remained "
                            "settled within the stability limits"
                        ),
                        feedback=tuple(feedback_samples),
                        metrics=metrics(settling_time),
                    )
            else:
                settled_since = None

            if (
                elapsed_after_step
                > trajectory.total_duration + active_policy.settle_timeout
            ):
                return ExecutionResult(
                    motion_name=trajectory.name,
                    backend=BACKEND_NAME,
                    status=ExecutionStatus.SETTLING_FAILED,
                    message=(
                        "Trajectory time elapsed but measured position and "
                        "velocity did not settle before the deadline"
                    ),
                    feedback=tuple(feedback_samples),
                    metrics=metrics(None),
                )

        if viewer is not None:
            remaining = model.opt.timestep - (time.perf_counter() - step_started)
            if remaining > 0:
                time.sleep(remaining)


def report_final_error(
    data: mujoco.MjData,
    mapping: MuJoCoJointMapping,
    trajectory: CompiledTrajectory,
) -> float:
    """Print final measured positions and return the largest absolute error."""

    if not isinstance(trajectory, CompiledTrajectory):
        raise TypeError("MuJoCo execution requires a Rust CompiledTrajectory")
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


def print_execution_summary(result: ExecutionResult) -> None:
    """Print a compact human-readable native MuJoCo result."""

    print(f"Result: {result.status.value}")
    print(f"Detail: {result.message}")
    if result.metrics is None:
        return
    metrics = result.metrics
    if metrics.maximum_position_errors:
        print(
            "Maximum tracking error: "
            f"{max(metrics.maximum_position_errors):.6f} rad"
        )
    if metrics.final_position_errors:
        print(
            "Maximum final position error: "
            f"{max(abs(value) for value in metrics.final_position_errors):.6f} rad"
        )
    if metrics.final_velocities:
        print(
            "Maximum final velocity: "
            f"{max(abs(value) for value in metrics.final_velocities):.6f} rad/s"
        )
    if metrics.settling_time is not None:
        print(f"Settling time: {metrics.settling_time:.3f} s")
    if metrics.maximum_base_translation is not None:
        print(
            "Maximum base translation: "
            f"{metrics.maximum_base_translation:.6f} m"
        )
        print(f"Maximum base tilt: {metrics.maximum_base_tilt:.6f} rad")
        print(
            "Longest base contact loss: "
            f"{metrics.longest_contact_loss:.3f} s"
        )


def play_motion(
    scene_path: Path,
    trajectory: CompiledTrajectory,
    start_positions: tuple[float, ...],
    *,
    lead_in: float,
    headless: bool,
    policy: StabilityPolicy | None = None,
) -> ExecutionResult:
    """Initialize MuJoCo and play one shared generated Orion trajectory."""

    if not isinstance(trajectory, CompiledTrajectory):
        raise TypeError("MuJoCo execution requires a Rust CompiledTrajectory")
    model = mujoco.MjModel.from_xml_path(str(scene_path))
    data = mujoco.MjData(model)
    mapping = resolve_joint_mapping(model, trajectory.joint_names)
    set_joint_state(model, data, mapping, start_positions)
    active_policy = policy or load_stability_policy()

    if headless:
        result = run_playback_loop(
            model,
            data,
            mapping,
            trajectory,
            lead_in=lead_in,
            viewer=None,
            policy=active_policy,
        )
    else:
        with mujoco.viewer.launch_passive(model, data) as viewer:
            viewer.cam.lookat[:] = [0.02, 0.0, 0.20]
            viewer.cam.distance = 0.75
            viewer.cam.azimuth = 90
            viewer.cam.elevation = -10
            result = run_playback_loop(
                model,
                data,
                mapping,
                trajectory,
                lead_in=lead_in,
                viewer=viewer,
                policy=active_policy,
            )

    print_execution_summary(result)
    return result


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
    parser.add_argument(
        "--report-json",
        type=Path,
        help="Optional path for the complete machine-readable run result.",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_arguments()
    try:
        motion_path, trajectory, start_positions = load_playback_data(
            args.motion, args.start_pose
        )
    except TrajectoryCompilerError as error:
        raise SystemExit(f"Cannot play motion: {error}") from None

    print(f"Motion: {trajectory.name}")
    print(f"Source: {motion_path}")
    print(f"Start pose: {args.start_pose}")
    print(f"Motion duration: {trajectory.total_duration:.3f} s")

    result = play_motion(
        args.scene.resolve(),
        trajectory,
        start_positions,
        lead_in=args.lead_in,
        headless=args.headless,
    )
    if args.report_json is not None:
        report_path = args.report_json.resolve()
        report_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.write_text(
            json.dumps(execution_result_data(result), indent=2) + "\n",
            encoding="utf-8",
        )
        print(f"Machine-readable report: {report_path}")
    if not result.succeeded:
        raise SystemExit(f"Playback failed: {result.status.value}")


if __name__ == "__main__":
    main()
