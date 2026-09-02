"""Consume trajectories compiled by Orion's Rust runtime.

This module deliberately contains no spline or keyframe implementation.  Its
job is to invoke the single Rust compiler, validate the versioned interchange
document, and expose fixed-rate samples to Python diagnostics.
"""

from __future__ import annotations

import bisect
import json
import math
import os
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any


PROJECT_ROOT = Path(__file__).resolve().parents[2]
RUNTIME_MANIFEST = PROJECT_ROOT / "runtime" / "Cargo.toml"
DEFAULT_POSE_FILE = PROJECT_ROOT / "motion" / "config" / "poses.yaml"
DEFAULT_MOTIONS_DIRECTORY = PROJECT_ROOT / "motion" / "motions"
DEFAULT_CALIBRATION_FILE = (
    PROJECT_ROOT / "simulation" / "mujoco" / "config" / "servo_calibration.json"
)
DEFAULT_CONTROL_RATE_HZ = 50.0
CANONICAL_JOINT_NAMES = (
    "base_yaw_joint",
    "shoulder_pitch_joint",
    "elbow_pitch_joint",
    "head_roll_joint",
    "head_pitch_joint",
)


class TrajectoryCompilerError(ValueError):
    """Raised when Rust cannot compile or export a requested motion."""


@dataclass(frozen=True)
class TrajectoryPoint:
    time_from_start: float
    positions: tuple[float, ...]
    velocities: tuple[float, ...]
    accelerations: tuple[float, ...]
    keyframe_index: int
    keyframe: str
    reached_markers: tuple[str, ...]


@dataclass(frozen=True)
class TrajectoryMarker:
    name: str
    time_seconds: float


@dataclass(frozen=True)
class JointRange:
    name: str
    lower_rad: float
    upper_rad: float


@dataclass(frozen=True)
class CompiledTrajectory:
    """A calibrated 50 Hz sample stream emitted by ``orion-runtime``."""

    name: str
    description: str
    space: str
    style: str
    joint_names: tuple[str, ...]
    duration_seconds: float
    control_rate_hz: float
    peak_velocity_rad_s: float
    amplitude_scale: float
    markers: tuple[TrajectoryMarker, ...]
    joint_ranges: tuple[JointRange, ...]
    samples: tuple[TrajectoryPoint, ...]
    hardware_profile: dict[str, Any]
    build_revision: str

    @property
    def points(self) -> tuple[TrajectoryPoint, ...]:
        """Compatibility name for consumers that operate on trajectory points."""

        return self.samples

    @property
    def total_duration(self) -> float:
        return self.duration_seconds


def _compiler_command() -> list[str]:
    configured = os.environ.get("ORION_TRAJECTORY_COMPILER")
    if configured:
        return [configured]
    binary = PROJECT_ROOT / "runtime" / "target" / "debug" / "orion-trajectory"
    if binary.is_file():
        return [str(binary)]
    return [
        "cargo",
        "run",
        "--quiet",
        "--manifest-path",
        str(RUNTIME_MANIFEST),
        "--bin",
        "orion-trajectory",
        "--",
    ]


def compile_trajectory(
    motion_name: str,
    start_pose_name: str,
    *,
    anchor_pose_name: str | None = None,
    pose_file: Path = DEFAULT_POSE_FILE,
    motions_directory: Path = DEFAULT_MOTIONS_DIRECTORY,
    calibration_file: Path = DEFAULT_CALIBRATION_FILE,
    control_rate_hz: float = DEFAULT_CONTROL_RATE_HZ,
) -> CompiledTrajectory:
    """Ask Rust to compile one named v2 motion from a named start pose."""

    command = [
        *_compiler_command(),
        "--motion",
        motion_name,
        "--start-pose",
        start_pose_name,
        "--pose-file",
        str(pose_file),
        "--motions-directory",
        str(motions_directory),
        "--calibration",
        str(calibration_file),
        "--control-rate-hz",
        str(control_rate_hz),
    ]
    if anchor_pose_name is not None:
        command.extend(("--anchor-pose", anchor_pose_name))
    completed = subprocess.run(
        command,
        cwd=PROJECT_ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise TrajectoryCompilerError(
            f"Rust trajectory compiler rejected '{motion_name}': {detail}"
        )
    try:
        document = json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise TrajectoryCompilerError(
            f"Rust trajectory compiler returned invalid JSON: {exc}"
        ) from exc
    return trajectory_from_document(document)


def trajectory_from_document(document: Any) -> CompiledTrajectory:
    """Validate and decode the v2 Rust trajectory interchange document."""

    if not isinstance(document, dict):
        raise TrajectoryCompilerError("Compiled trajectory must be a JSON object.")
    if document.get("format_version") != 2 or document.get("compiler") != "orion-runtime":
        raise TrajectoryCompilerError(
            "Compiled trajectory must use format_version 2 from orion-runtime."
        )
    joint_names = _string_tuple(document.get("joint_names"), "joint_names")
    if joint_names != CANONICAL_JOINT_NAMES:
        raise TrajectoryCompilerError(
            "Compiled trajectory joint order does not match Orion."
        )
    duration = _finite_number(document.get("duration_seconds"), "duration_seconds")
    control_rate = _finite_number(document.get("control_rate_hz"), "control_rate_hz")
    peak_velocity = _finite_number(
        document.get("peak_velocity_rad_s"), "peak_velocity_rad_s"
    )
    amplitude_scale = _finite_number(
        document.get("amplitude_scale"), "amplitude_scale"
    )
    if duration <= 0 or control_rate <= 0 or peak_velocity < 0:
        raise TrajectoryCompilerError(
            "Compiled duration/control rate must be positive and peak velocity non-negative."
        )
    if not 0 <= amplitude_scale <= 1:
        raise TrajectoryCompilerError("Compiled amplitude scale must be within 0..1.")

    raw_samples = document.get("samples")
    if not isinstance(raw_samples, list) or len(raw_samples) < 2:
        raise TrajectoryCompilerError("Compiled trajectory requires at least two samples.")
    samples = tuple(_sample_from_document(item, len(joint_names)) for item in raw_samples)
    times = tuple(sample.time_from_start for sample in samples)
    if times[0] != 0.0 or any(right <= left for left, right in zip(times, times[1:])):
        raise TrajectoryCompilerError(
            "Compiled sample times must start at zero and increase strictly."
        )
    if not math.isclose(times[-1], duration, rel_tol=0.0, abs_tol=1e-9):
        raise TrajectoryCompilerError(
            "Compiled trajectory must include its exact final sample."
        )

    markers = tuple(
        TrajectoryMarker(
            name=_string(item.get("name"), "markers[].name"),
            time_seconds=_finite_number(
                item.get("time_seconds"), "markers[].time_seconds"
            ),
        )
        for item in _object_list(document.get("markers"), "markers")
    )
    ranges = tuple(
        JointRange(
            name=_string(item.get("name"), "joint_ranges[].name"),
            lower_rad=_finite_number(item.get("lower_rad"), "joint_ranges[].lower_rad"),
            upper_rad=_finite_number(item.get("upper_rad"), "joint_ranges[].upper_rad"),
        )
        for item in _object_list(document.get("joint_ranges"), "joint_ranges")
    )
    if tuple(item.name for item in ranges) != joint_names:
        raise TrajectoryCompilerError("Compiled calibration ranges do not match Orion.")
    for sample in samples:
        for value, joint_range in zip(sample.positions, ranges, strict=True):
            if not joint_range.lower_rad - 1e-9 <= value <= joint_range.upper_rad + 1e-9:
                raise TrajectoryCompilerError(
                    f"Compiled {joint_range.name} sample is outside calibration."
                )

    hardware_profile = document.get("hardware_profile")
    if not isinstance(hardware_profile, dict):
        raise TrajectoryCompilerError("Compiled trajectory omits hardware_profile.")
    if hardware_profile.get("variant") != "7.4 V STS3215":
        raise TrajectoryCompilerError("Compiled trajectory has the wrong motor profile.")

    return CompiledTrajectory(
        name=_string(document.get("motion_name"), "motion_name"),
        description=_string(document.get("description"), "description", empty=True),
        space=_string(document.get("space"), "space"),
        style=_string(document.get("style"), "style"),
        joint_names=joint_names,
        duration_seconds=duration,
        control_rate_hz=control_rate,
        peak_velocity_rad_s=peak_velocity,
        amplitude_scale=amplitude_scale,
        markers=markers,
        joint_ranges=ranges,
        samples=samples,
        hardware_profile=hardware_profile,
        build_revision=_string(document.get("build_revision"), "build_revision"),
    )


def sample_trajectory(
    trajectory: CompiledTrajectory, elapsed_seconds: float
) -> tuple[TrajectoryPoint, int]:
    """Return the command held at ``elapsed_seconds`` on the 50 Hz grid."""

    if not isinstance(trajectory, CompiledTrajectory):
        raise TypeError("Expected a Rust CompiledTrajectory.")
    if not math.isfinite(elapsed_seconds):
        raise ValueError("Elapsed trajectory time must be finite.")
    times = [sample.time_from_start for sample in trajectory.samples]
    index = bisect.bisect_right(times, max(0.0, elapsed_seconds)) - 1
    index = min(max(index, 0), len(trajectory.samples) - 1)
    return trajectory.samples[index], index


def _sample_from_document(document: Any, joint_count: int) -> TrajectoryPoint:
    if not isinstance(document, dict):
        raise TrajectoryCompilerError("Each compiled sample must be an object.")
    return TrajectoryPoint(
        time_from_start=_finite_number(
            document.get("time_from_start"), "samples[].time_from_start"
        ),
        positions=_number_tuple(document.get("positions"), joint_count, "positions"),
        velocities=_number_tuple(document.get("velocities"), joint_count, "velocities"),
        accelerations=_number_tuple(
            document.get("accelerations"), joint_count, "accelerations"
        ),
        keyframe_index=_nonnegative_integer(
            document.get("keyframe_index"), "samples[].keyframe_index"
        ),
        keyframe=_string(document.get("keyframe"), "samples[].keyframe"),
        reached_markers=_string_tuple(
            document.get("reached_markers"), "samples[].reached_markers"
        ),
    )


def _finite_number(value: Any, path: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TrajectoryCompilerError(f"{path} must be a finite number.")
    converted = float(value)
    if not math.isfinite(converted):
        raise TrajectoryCompilerError(f"{path} must be a finite number.")
    return converted


def _number_tuple(value: Any, length: int, path: str) -> tuple[float, ...]:
    if not isinstance(value, list) or len(value) != length:
        raise TrajectoryCompilerError(f"{path} must contain {length} numbers.")
    return tuple(_finite_number(item, f"{path}[]") for item in value)


def _string(value: Any, path: str, *, empty: bool = False) -> str:
    if not isinstance(value, str) or (not empty and not value):
        raise TrajectoryCompilerError(f"{path} must be a string.")
    return value


def _string_tuple(value: Any, path: str) -> tuple[str, ...]:
    if not isinstance(value, list):
        raise TrajectoryCompilerError(f"{path} must be a list of strings.")
    return tuple(_string(item, f"{path}[]") for item in value)


def _object_list(value: Any, path: str) -> list[dict[str, Any]]:
    if not isinstance(value, list) or any(not isinstance(item, dict) for item in value):
        raise TrajectoryCompilerError(f"{path} must be a list of objects.")
    return value


def _nonnegative_integer(value: Any, path: str) -> int:
    if type(value) is not int or value < 0:
        raise TrajectoryCompilerError(f"{path} must be a non-negative integer.")
    return value
