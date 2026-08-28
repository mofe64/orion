"""Offline static gravity-torque analysis for Orion's native MuJoCo model.

The generated MJCF is rooted in the articulated arm and reaches the physical
base through the shoulder and base-yaw joints.  MuJoCo inverse dynamics therefore
needs a base-support correction: the free-root wrench is transferred to the
physical base body before the five actuator torques are reported.

This tool does not command actuators and does not use the MJCF actuator force
range as evidence of physical motor capability.
"""

from __future__ import annotations

import argparse
import math
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence

import mujoco
import numpy as np
import yaml

from mujoco_backend import (
    DEFAULT_BASE_BODY_NAME,
    MuJoCoJointMapping,
    resolve_joint_mapping,
    set_joint_state,
)


MUJOCO_DIRECTORY = Path(__file__).resolve().parent
PROJECT_ROOT = MUJOCO_DIRECTORY.parents[1]
MOTION_PACKAGE_SOURCE = PROJECT_ROOT / "ros2_ws/src/orion_motion"
CONFIG_DIRECTORY = MOTION_PACKAGE_SOURCE / "config"
DEFAULT_SCENE_PATH = MUJOCO_DIRECTORY / "scene.xml"
DEFAULT_POSE_PATH = CONFIG_DIRECTORY / "poses.yaml"
DEFAULT_POSE_NAMES = ("rest", "zero_reference", "home", "attentive")

# Consume the motion package from this source tree, matching motion_player.py.
sys.path.insert(0, str(MOTION_PACKAGE_SOURCE))

from orion_motion.motion_loader import load_yaml_file  # noqa: E402
from orion_motion.trajectory_builder import build_pose_trajectory  # noqa: E402
from orion_motion.trajectory_generator import (  # noqa: E402
    generate_trajectory,
    sample_trajectory,
)
from orion_motion.trajectory_validator import (  # noqa: E402
    ValidatedTrajectory,
    require_valid_trajectory,
)

JOINT_NAMES = (
    "base_yaw_joint",
    "shoulder_pitch_joint",
    "elbow_pitch_joint",
    "head_roll_joint",
    "head_pitch_joint",
)

# Published STS3215 reference values at 6 V. These are comparison thresholds,
# not a mapping from the servo's raw Torque_Limit register.
RATED_TORQUE_NM = 0.39
STALL_TORQUE_NM = 1.62

# Goal_Velocity=50 in the commissioning runner is approximately 4.4 degrees/s.
# This comparison is intentionally separate from ROS motion limits.
COMMISSIONING_VELOCITY_LIMIT_RAD_S = math.radians(4.4)


class TorqueAnalysisError(RuntimeError):
    """Raised when a pose cannot produce a trustworthy static report."""


@dataclass(frozen=True)
class JointTorqueDemand:
    """One modelled static joint-torque requirement."""

    joint_name: str
    torque_nm: float

    @property
    def absolute_torque_nm(self) -> float:
        return abs(self.torque_nm)

    @property
    def rated_fraction(self) -> float:
        return self.absolute_torque_nm / RATED_TORQUE_NM

    @property
    def stall_fraction(self) -> float:
        return self.absolute_torque_nm / STALL_TORQUE_NM


@dataclass(frozen=True)
class StaticPoseTorqueReport:
    """Static inverse-dynamics evidence for one named pose."""

    pose_name: str
    joint_demands: tuple[JointTorqueDemand, ...]
    base_support_force_n: tuple[float, float, float]
    base_support_moment_nm: tuple[float, float, float]


@dataclass(frozen=True)
class DynamicJointTorqueDemand:
    """Peak and sustained model demand for one joint over a trajectory."""

    joint_name: str
    peak_torque_nm: float
    peak_time_seconds: float
    rms_torque_nm: float
    peak_velocity_rad_s: float
    peak_acceleration_rad_s2: float

    @property
    def peak_absolute_torque_nm(self) -> float:
        return abs(self.peak_torque_nm)

    @property
    def rated_fraction(self) -> float:
        return self.peak_absolute_torque_nm / RATED_TORQUE_NM

    @property
    def stall_fraction(self) -> float:
        return self.peak_absolute_torque_nm / STALL_TORQUE_NM

    @property
    def commissioning_velocity_fraction(self) -> float:
        return self.peak_velocity_rad_s / COMMISSIONING_VELOCITY_LIMIT_RAD_S


@dataclass(frozen=True)
class DynamicTrajectoryTorqueReport:
    """Sampled inverse-dynamics evidence for one validated trajectory."""

    trajectory_name: str
    start_pose_name: str
    duration_seconds: float
    sample_period_seconds: float
    sample_count: int
    joint_demands: tuple[DynamicJointTorqueDemand, ...]

    @property
    def minimum_duration_for_velocity_setting_seconds(self) -> float:
        """Duration needed to fit the same quintic path under the servo setting."""

        maximum_ratio = max(
            demand.commissioning_velocity_fraction
            for demand in self.joint_demands
        )
        return self.duration_seconds * maximum_ratio


def load_named_pose(
    pose_path: Path,
    pose_name: str,
    joint_names: Sequence[str] = JOINT_NAMES,
) -> tuple[float, ...]:
    """Load one complete, finite named pose in canonical joint order."""

    try:
        root = yaml.safe_load(pose_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, yaml.YAMLError) as exc:
        raise TorqueAnalysisError(f"Could not read poses '{pose_path}': {exc}") from exc

    if not isinstance(root, dict) or root.get("format_version") != 1:
        raise TorqueAnalysisError("Pose library must use format_version 1.")
    if root.get("units") != "radians":
        raise TorqueAnalysisError("Pose library must use radians.")
    poses = root.get("poses")
    pose = poses.get(pose_name) if isinstance(poses, dict) else None
    positions = pose.get("positions") if isinstance(pose, dict) else None
    if not isinstance(positions, dict):
        raise TorqueAnalysisError(f"Pose library has no complete pose '{pose_name}'.")

    expected = set(joint_names)
    if set(positions) != expected:
        missing = ", ".join(sorted(expected - set(positions))) or "none"
        unexpected = ", ".join(sorted(set(positions) - expected)) or "none"
        raise TorqueAnalysisError(
            f"Pose '{pose_name}' joint mismatch; missing: {missing}; "
            f"unexpected: {unexpected}."
        )

    values: list[float] = []
    for joint_name in joint_names:
        value = positions[joint_name]
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise TorqueAnalysisError(
                f"Pose '{pose_name}' {joint_name} must be numeric."
            )
        numeric = float(value)
        if not math.isfinite(numeric):
            raise TorqueAnalysisError(
                f"Pose '{pose_name}' {joint_name} must be finite."
            )
        values.append(numeric)
    return tuple(values)


def require_positions_within_model_limits(
    model: mujoco.MjModel,
    mapping: MuJoCoJointMapping,
    positions: Sequence[float],
) -> None:
    """Reject analysis of a pose that the loaded MJCF considers illegal."""

    for joint_name, position in zip(mapping.joint_names, positions, strict=True):
        joint_id = mujoco.mj_name2id(
            model, mujoco.mjtObj.mjOBJ_JOINT, joint_name
        )
        if not model.jnt_limited[joint_id]:
            continue
        lower, upper = (float(value) for value in model.jnt_range[joint_id])
        if not lower <= position <= upper:
            raise TorqueAnalysisError(
                f"Pose requests {joint_name}={position:+.6f} rad outside "
                f"MuJoCo range [{lower:+.6f}, {upper:+.6f}]."
            )


def _free_dof_addresses(model: mujoco.MjModel) -> tuple[int, ...]:
    free_joint_ids = [
        joint_id
        for joint_id in range(model.njnt)
        if model.jnt_type[joint_id] == mujoco.mjtJoint.mjJNT_FREE
    ]
    if len(free_joint_ids) != 1:
        raise TorqueAnalysisError(
            "Base-support analysis requires exactly one free joint."
        )
    first_address = int(model.jnt_dofadr[free_joint_ids[0]])
    return tuple(range(first_address, first_address + 6))


def _base_support_wrench(
    model: mujoco.MjModel,
    data: mujoco.MjData,
    inverse_force: np.ndarray,
    *,
    base_body_name: str,
) -> tuple[np.ndarray, np.ndarray]:
    """Transfer the inverse-dynamics free-root wrench to the physical base.

    Returns the six-component base wrench (force then moment) and its generalized
    force contribution. Subtracting that contribution leaves the actuator force
    needed when the real base, rather than the generated model root, is supported.
    """

    base_body_id = mujoco.mj_name2id(
        model, mujoco.mjtObj.mjOBJ_BODY, base_body_name
    )
    if base_body_id < 0:
        raise TorqueAnalysisError(
            f"MuJoCo model has no physical base body '{base_body_name}'."
        )

    translation_jacobian = np.zeros((3, model.nv))
    rotation_jacobian = np.zeros((3, model.nv))
    mujoco.mj_jacBody(
        model,
        data,
        translation_jacobian,
        rotation_jacobian,
        base_body_id,
    )
    base_jacobian = np.vstack((translation_jacobian, rotation_jacobian))
    free_addresses = _free_dof_addresses(model)
    free_mapping = base_jacobian[:, free_addresses].T

    try:
        wrench = np.linalg.solve(free_mapping, inverse_force[list(free_addresses)])
    except np.linalg.LinAlgError as exc:
        raise TorqueAnalysisError(
            "Physical-base Jacobian could not resolve the free-root support wrench."
        ) from exc
    generalized_support = base_jacobian.T @ wrench
    residual = inverse_force[list(free_addresses)] - generalized_support[
        list(free_addresses)
    ]
    if float(np.max(np.abs(residual))) > 1e-9:
        raise TorqueAnalysisError(
            "Base-support correction left an unexpected free-root force residual."
        )
    return wrench, generalized_support


def _base_jacobian(
    model: mujoco.MjModel,
    data: mujoco.MjData,
    base_body_id: int,
) -> np.ndarray:
    """Return the physical base's translational-then-rotational Jacobian."""

    translation_jacobian = np.zeros((3, model.nv))
    rotation_jacobian = np.zeros((3, model.nv))
    mujoco.mj_jacBody(
        model,
        data,
        translation_jacobian,
        rotation_jacobian,
        base_body_id,
    )
    return np.vstack((translation_jacobian, rotation_jacobian))


def _base_jacobian_at_position(
    model: mujoco.MjModel,
    qpos: np.ndarray,
    base_body_id: int,
) -> np.ndarray:
    data = mujoco.MjData(model)
    data.qpos[:] = qpos
    mujoco.mj_fwdPosition(model, data)
    return _base_jacobian(model, data, base_body_id)


def _set_base_fixed_dynamic_state(
    model: mujoco.MjModel,
    data: mujoco.MjData,
    mapping: MuJoCoJointMapping,
    velocities: Sequence[float],
    accelerations: Sequence[float],
    *,
    base_body_name: str,
) -> None:
    """Set trajectory derivatives while holding the physical base fixed."""

    velocity_values = tuple(float(value) for value in velocities)
    acceleration_values = tuple(float(value) for value in accelerations)
    expected = len(mapping.joint_names)
    if len(velocity_values) != expected or len(acceleration_values) != expected:
        raise TorqueAnalysisError(
            "Trajectory velocity and acceleration vectors must match Orion's joints."
        )

    base_body_id = mujoco.mj_name2id(
        model, mujoco.mjtObj.mjOBJ_BODY, base_body_name
    )
    if base_body_id < 0:
        raise TorqueAnalysisError(
            f"MuJoCo model has no physical base body '{base_body_name}'."
        )
    free_addresses = _free_dof_addresses(model)
    base_jacobian = _base_jacobian(model, data, base_body_id)
    free_jacobian = base_jacobian[:, free_addresses]

    data.qvel[:] = 0.0
    for address, value in zip(
        mapping.dof_addresses, velocity_values, strict=True
    ):
        data.qvel[address] = value
    try:
        data.qvel[list(free_addresses)] = np.linalg.solve(
            free_jacobian, -base_jacobian @ data.qvel
        )
    except np.linalg.LinAlgError as exc:
        raise TorqueAnalysisError(
            "Physical-base Jacobian could not resolve fixed-base velocity."
        ) from exc
    mujoco.mj_fwdVelocity(model, data)

    # Jdot*qvel is evaluated by a centered directional derivative along the
    # current generalized velocity. It captures the centripetal/Coriolis part
    # of the base acceleration without assuming the generated tree is base-rooted.
    derivative_step = 1e-6
    qpos_forward = data.qpos.copy()
    qpos_backward = data.qpos.copy()
    mujoco.mj_integratePos(
        model, qpos_forward, data.qvel, derivative_step
    )
    mujoco.mj_integratePos(
        model, qpos_backward, data.qvel, -derivative_step
    )
    jacobian_dot = (
        _base_jacobian_at_position(model, qpos_forward, base_body_id)
        - _base_jacobian_at_position(model, qpos_backward, base_body_id)
    ) / (2.0 * derivative_step)

    data.qacc[:] = 0.0
    for address, value in zip(
        mapping.dof_addresses, acceleration_values, strict=True
    ):
        data.qacc[address] = value
    acceleration_residual = (
        base_jacobian @ data.qacc + jacobian_dot @ data.qvel
    )
    try:
        data.qacc[list(free_addresses)] = np.linalg.solve(
            free_jacobian, -acceleration_residual
        )
    except np.linalg.LinAlgError as exc:
        raise TorqueAnalysisError(
            "Physical-base Jacobian could not resolve fixed-base acceleration."
        ) from exc

    final_residual = base_jacobian @ data.qacc + jacobian_dot @ data.qvel
    if float(np.max(np.abs(final_residual))) > 1e-8:
        raise TorqueAnalysisError(
            "Fixed-base trajectory state left an acceleration residual."
        )


def analyze_static_pose(
    model: mujoco.MjModel,
    pose_name: str,
    positions: Sequence[float],
    *,
    base_body_name: str = DEFAULT_BASE_BODY_NAME,
) -> StaticPoseTorqueReport:
    """Compute static gravity-hold torque for one legal Orion pose."""

    mapping = resolve_joint_mapping(model, JOINT_NAMES)
    values = tuple(float(value) for value in positions)
    if len(values) != len(mapping.joint_names):
        raise TorqueAnalysisError(
            f"Expected {len(mapping.joint_names)} positions, got {len(values)}."
        )
    require_positions_within_model_limits(model, mapping, values)

    data = mujoco.MjData(model)
    set_joint_state(model, data, mapping, values)
    data.qvel[:] = 0.0
    data.qacc[:] = 0.0
    data.ctrl[:] = 0.0
    data.qfrc_applied[:] = 0.0
    data.xfrc_applied[:] = 0.0
    mujoco.mj_inverse(model, data)

    inverse_force = data.qfrc_inverse.copy()
    support_wrench, generalized_support = _base_support_wrench(
        model,
        data,
        inverse_force,
        base_body_name=base_body_name,
    )
    supported_force = inverse_force - generalized_support
    joint_demands = tuple(
        JointTorqueDemand(joint_name, float(supported_force[dof_address]))
        for joint_name, dof_address in zip(
            mapping.joint_names, mapping.dof_addresses, strict=True
        )
    )
    return StaticPoseTorqueReport(
        pose_name=pose_name,
        joint_demands=joint_demands,
        base_support_force_n=tuple(float(value) for value in support_wrench[:3]),
        base_support_moment_nm=tuple(float(value) for value in support_wrench[3:]),
    )


def analyze_named_poses(
    scene_path: Path,
    pose_path: Path,
    pose_names: Sequence[str],
) -> tuple[StaticPoseTorqueReport, ...]:
    """Load one model and analyze named poses without stepping simulation."""

    try:
        model = mujoco.MjModel.from_xml_path(str(scene_path))
    except ValueError as exc:
        raise TorqueAnalysisError(
            f"Could not compile MuJoCo scene '{scene_path}': {exc}"
        ) from exc

    return tuple(
        analyze_static_pose(
            model,
            pose_name,
            load_named_pose(pose_path, pose_name),
        )
        for pose_name in pose_names
    )


def load_pose_trajectory(
    pose_name: str,
    start_pose_name: str,
    duration_seconds: float,
    *,
    config_directory: Path = CONFIG_DIRECTORY,
) -> ValidatedTrajectory:
    """Build and validate one stopped-start named-pose trajectory."""

    if not math.isfinite(duration_seconds) or duration_seconds <= 0.0:
        raise TorqueAnalysisError("Trajectory duration must be finite and positive.")
    pose_path = config_directory / "poses.yaml"
    pose_library = load_yaml_file(pose_path)
    motion_limits = load_yaml_file(config_directory / "motion_limits.yaml")
    forbidden_regions = load_yaml_file(
        config_directory / "forbidden_regions.yaml"
    )
    requested = build_pose_trajectory(
        pose_name,
        duration_seconds,
        pose_library,
        motion_limits,
    )
    start_positions = load_named_pose(pose_path, start_pose_name)
    generated = generate_trajectory(
        requested,
        start_positions,
        (0.0,) * len(start_positions),
        motion_limits,
    )
    return require_valid_trajectory(
        generated,
        motion_limits,
        forbidden_regions,
    )


def analyze_dynamic_trajectory(
    model: mujoco.MjModel,
    validated: ValidatedTrajectory,
    *,
    start_pose_name: str,
    sample_period_seconds: float = 0.01,
    base_body_name: str = DEFAULT_BASE_BODY_NAME,
) -> DynamicTrajectoryTorqueReport:
    """Sample one validated trajectory and report peak/RMS inverse dynamics."""

    if not isinstance(validated, ValidatedTrajectory):
        raise TypeError("Dynamic torque analysis requires a ValidatedTrajectory.")
    if (
        not math.isfinite(sample_period_seconds)
        or sample_period_seconds <= 0.0
    ):
        raise TorqueAnalysisError("Sample period must be finite and positive.")

    trajectory = validated.trajectory
    if trajectory.joint_names != JOINT_NAMES:
        raise TorqueAnalysisError(
            "Trajectory joint order does not match Orion's canonical joint order."
        )
    if trajectory.total_duration <= 0.0:
        raise TorqueAnalysisError("Trajectory must have positive duration.")

    mapping = resolve_joint_mapping(model, trajectory.joint_names)
    intervals = max(
        1, math.ceil(trajectory.total_duration / sample_period_seconds)
    )
    times = np.linspace(0.0, trajectory.total_duration, intervals + 1)
    torque_samples: list[tuple[float, ...]] = []
    velocity_samples: list[tuple[float, ...]] = []
    acceleration_samples: list[tuple[float, ...]] = []

    for elapsed in times:
        point, _ = sample_trajectory(trajectory, float(elapsed))
        require_positions_within_model_limits(model, mapping, point.positions)
        data = mujoco.MjData(model)
        set_joint_state(model, data, mapping, point.positions)
        _set_base_fixed_dynamic_state(
            model,
            data,
            mapping,
            point.velocities,
            point.accelerations,
            base_body_name=base_body_name,
        )
        data.ctrl[:] = 0.0
        data.qfrc_applied[:] = 0.0
        data.xfrc_applied[:] = 0.0
        mujoco.mj_inverse(model, data)
        inverse_force = data.qfrc_inverse.copy()
        _, generalized_support = _base_support_wrench(
            model,
            data,
            inverse_force,
            base_body_name=base_body_name,
        )
        supported_force = inverse_force - generalized_support
        torque_samples.append(
            tuple(float(supported_force[address]) for address in mapping.dof_addresses)
        )
        velocity_samples.append(tuple(float(value) for value in point.velocities))
        acceleration_samples.append(
            tuple(float(value) for value in point.accelerations)
        )

    torque_matrix = np.asarray(torque_samples)
    velocity_matrix = np.asarray(velocity_samples)
    acceleration_matrix = np.asarray(acceleration_samples)
    demands: list[DynamicJointTorqueDemand] = []
    for joint_index, joint_name in enumerate(mapping.joint_names):
        torques = torque_matrix[:, joint_index]
        peak_index = int(np.argmax(np.abs(torques)))
        rms_torque = math.sqrt(
            float(np.trapezoid(torques**2, times) / trajectory.total_duration)
        )
        demands.append(
            DynamicJointTorqueDemand(
                joint_name=joint_name,
                peak_torque_nm=float(torques[peak_index]),
                peak_time_seconds=float(times[peak_index]),
                rms_torque_nm=rms_torque,
                peak_velocity_rad_s=float(
                    np.max(np.abs(velocity_matrix[:, joint_index]))
                ),
                peak_acceleration_rad_s2=float(
                    np.max(np.abs(acceleration_matrix[:, joint_index]))
                ),
            )
        )

    return DynamicTrajectoryTorqueReport(
        trajectory_name=trajectory.name,
        start_pose_name=start_pose_name,
        duration_seconds=trajectory.total_duration,
        sample_period_seconds=trajectory.total_duration / intervals,
        sample_count=len(times),
        joint_demands=tuple(demands),
    )


def _status(demand: JointTorqueDemand) -> str:
    if demand.absolute_torque_nm > STALL_TORQUE_NM:
        return "OVER STALL"
    if demand.absolute_torque_nm > RATED_TORQUE_NM:
        return "over rated"
    return "within rated"


def format_report(report: StaticPoseTorqueReport) -> str:
    """Format one compact terminal report."""

    force = report.base_support_force_n
    moment = report.base_support_moment_nm
    lines = [
        f"Pose: {report.pose_name}",
        (
            "Base support: "
            f"force=({force[0]:+.3f}, {force[1]:+.3f}, {force[2]:+.3f}) N, "
            f"moment=({moment[0]:+.3f}, {moment[1]:+.3f}, {moment[2]:+.3f}) N.m"
        ),
        "joint                       torque N.m   rated   stall   assessment",
    ]
    for demand in report.joint_demands:
        lines.append(
            f"{demand.joint_name:27} {demand.torque_nm:+10.4f} "
            f"{demand.rated_fraction:7.0%} {demand.stall_fraction:7.0%}   "
            f"{_status(demand)}"
        )
    return "\n".join(lines)


def _dynamic_status(demand: DynamicJointTorqueDemand) -> str:
    findings: list[str] = []
    if demand.peak_absolute_torque_nm > STALL_TORQUE_NM:
        findings.append("OVER STALL")
    elif demand.peak_absolute_torque_nm > RATED_TORQUE_NM:
        findings.append("torque>rated")
    if demand.commissioning_velocity_fraction > 1.0:
        findings.append("speed>setting")
    return ", ".join(findings) or "within references"


def format_dynamic_report(report: DynamicTrajectoryTorqueReport) -> str:
    """Format one sampled trajectory report for terminal review."""

    lines = [
        (
            f"Trajectory: {report.trajectory_name} from {report.start_pose_name} "
            f"({report.duration_seconds:.3f} s, {report.sample_count} samples at "
            f"{report.sample_period_seconds:.4f} s)"
        ),
        (
            "joint                    peak N.m  at s   RMS N.m  peak speed  "
            "speed/set  assessment"
        ),
    ]
    for demand in report.joint_demands:
        speed_degrees = math.degrees(demand.peak_velocity_rad_s)
        lines.append(
            f"{demand.joint_name:24} {demand.peak_torque_nm:+9.4f} "
            f"{demand.peak_time_seconds:5.2f} {demand.rms_torque_nm:9.4f} "
            f"{speed_degrees:9.2f}°/s "
            f"{demand.commissioning_velocity_fraction:8.1f}x  "
            f"{_dynamic_status(demand)}"
        )
    lines.append(
        "Minimum same-path duration for the approximate commissioning velocity "
        f"setting: {report.minimum_duration_for_velocity_setting_seconds:.2f} s"
    )
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Report Orion static gravity-hold torque from MuJoCo."
    )
    parser.add_argument(
        "--scene",
        type=Path,
        default=DEFAULT_SCENE_PATH,
        help="MuJoCo scene XML path.",
    )
    parser.add_argument(
        "--poses",
        type=Path,
        default=DEFAULT_POSE_PATH,
        help="Shared Orion pose-library path.",
    )
    parser.add_argument(
        "--pose",
        action="append",
        dest="pose_names",
        help="Pose to analyze; repeat for multiple poses.",
    )
    parser.add_argument(
        "--trajectory-to",
        help="Analyze a generated named-pose trajectory instead of static poses.",
    )
    parser.add_argument(
        "--start-pose",
        default="rest",
        help="Stopped starting pose for trajectory analysis (default: rest).",
    )
    parser.add_argument(
        "--duration",
        type=float,
        default=6.0,
        help="Named-pose trajectory duration in seconds (default: 6.0).",
    )
    parser.add_argument(
        "--sample-period",
        type=float,
        default=0.01,
        help="Dynamic analysis sample period in seconds (default: 0.01).",
    )
    return parser


def main() -> int:
    args = build_parser().parse_args()
    try:
        if args.trajectory_to:
            model = mujoco.MjModel.from_xml_path(str(args.scene))
            validated = load_pose_trajectory(
                args.trajectory_to,
                args.start_pose,
                args.duration,
                config_directory=args.poses.parent,
            )
            dynamic_report = analyze_dynamic_trajectory(
                model,
                validated,
                start_pose_name=args.start_pose,
                sample_period_seconds=args.sample_period,
            )
            reports: tuple[StaticPoseTorqueReport, ...] = ()
        else:
            pose_names = tuple(args.pose_names or DEFAULT_POSE_NAMES)
            reports = analyze_named_poses(args.scene, args.poses, pose_names)
            dynamic_report = None
    except (TorqueAnalysisError, ValueError) as exc:
        raise SystemExit(f"Torque analysis failed: {exc}") from exc

    print(
        "MODEL-BASED ESTIMATE: verify assembled masses/centres of mass and do not "
        "map N.m directly to STS3215 Torque_Limit raw values."
    )
    if dynamic_report is not None:
        print(format_dynamic_report(dynamic_report))
    else:
        for index, report in enumerate(reports):
            if index:
                print()
            print(format_report(report))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
