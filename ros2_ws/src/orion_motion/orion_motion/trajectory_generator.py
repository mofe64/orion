"""Generate smooth backend-neutral trajectories from resolved keyframes."""

from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Any, Sequence

from orion_motion.motion_validator import validate_motion_limits
from orion_motion.trajectory_builder import ResolvedTrajectory


_PEAK_VELOCITY_FACTOR = 15.0 / 8.0
_PEAK_ACCELERATION_FACTOR = 10.0 / math.sqrt(3.0)
_PEAK_JERK_FACTOR = 60.0


class TrajectoryGenerationError(ValueError):
    """Raised when measured state or generated dynamics are not executable."""


@dataclass(frozen=True)
class TrajectoryPoint:
    """One complete desired joint state at an absolute trajectory time."""

    time_from_start: float
    positions: tuple[float, ...]
    velocities: tuple[float, ...]
    accelerations: tuple[float, ...]


@dataclass(frozen=True)
class GeneratedSegment:
    """One quintic transition or constant-position hold."""

    pose_name: str
    kind: str
    start: TrajectoryPoint
    end: TrajectoryPoint

    @property
    def duration(self) -> float:
        return self.end.time_from_start - self.start.time_from_start


@dataclass(frozen=True)
class JointPeakDynamics:
    """Analytic peak magnitudes for one joint during one segment."""

    segment_index: int
    pose_name: str
    joint_name: str
    velocity: float
    acceleration: float
    jerk: float


@dataclass(frozen=True)
class GeneratedTrajectory:
    """A measured-start trajectory shared by every execution backend."""

    name: str
    description: str | None
    joint_names: tuple[str, ...]
    points: tuple[TrajectoryPoint, ...]
    segments: tuple[GeneratedSegment, ...]
    peak_dynamics: tuple[JointPeakDynamics, ...]
    total_duration: float


def _finite_vector(
    values: Sequence[float], expected_length: int, path: str
) -> tuple[float, ...]:
    if len(values) != expected_length:
        raise TrajectoryGenerationError(
            f"{path} must contain {expected_length} values, "
            f"got {len(values)}"
        )

    converted: list[float] = []
    for index, value in enumerate(values):
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise TrajectoryGenerationError(
                f"{path}[{index}] must be a finite number"
            )
        converted_value = float(value)
        if not math.isfinite(converted_value):
            raise TrajectoryGenerationError(
                f"{path}[{index}] must be a finite number"
            )
        converted.append(converted_value)
    return tuple(converted)


def _stationary_point(
    time_from_start: float, positions: tuple[float, ...]
) -> TrajectoryPoint:
    zeros = (0.0,) * len(positions)
    return TrajectoryPoint(
        time_from_start=time_from_start,
        positions=positions,
        velocities=zeros,
        accelerations=zeros,
    )


def _segment_peaks(
    segment_index: int,
    pose_name: str,
    joint_names: tuple[str, ...],
    start_positions: tuple[float, ...],
    end_positions: tuple[float, ...],
    duration: float,
) -> tuple[JointPeakDynamics, ...]:
    peaks = []
    for joint_name, start, end in zip(
        joint_names, start_positions, end_positions, strict=True
    ):
        displacement = abs(end - start)
        peaks.append(
            JointPeakDynamics(
                segment_index=segment_index,
                pose_name=pose_name,
                joint_name=joint_name,
                velocity=_PEAK_VELOCITY_FACTOR * displacement / duration,
                acceleration=(
                    _PEAK_ACCELERATION_FACTOR * displacement / duration**2
                ),
                jerk=_PEAK_JERK_FACTOR * displacement / duration**3,
            )
        )
    return tuple(peaks)


def _validate_peak_dynamics(
    peaks: Sequence[JointPeakDynamics], limits: dict[str, Any]
) -> None:
    limit_fields = (
        ("velocity", "max_velocity"),
        ("acceleration", "max_acceleration"),
        ("jerk", "max_jerk"),
    )
    for peak in peaks:
        joint_limits = limits["joints"][peak.joint_name]
        for measured_field, limit_field in limit_fields:
            measured = getattr(peak, measured_field)
            allowed = float(joint_limits[limit_field])
            if measured > allowed + 1e-12:
                raise TrajectoryGenerationError(
                    f"transition to pose '{peak.pose_name}' segment "
                    f"{peak.segment_index} "
                    f"exceeds {peak.joint_name} {limit_field}: "
                    f"peak {measured:.6f}, limit {allowed:.6f}"
                )


def generate_trajectory(
    requested: ResolvedTrajectory,
    measured_positions: Sequence[float],
    measured_velocities: Sequence[float],
    motion_limits: Any,
) -> GeneratedTrajectory:
    """Generate and dynamically validate a stopped-start quintic trajectory.

    Every transition uses the minimum-jerk quintic time scaling
    ``10u^3 - 15u^4 + 6u^5``. Position, velocity, and acceleration are zero-
    derivative matched at authored keyframes and holds. Moving-state blending
    is intentionally rejected until preemption and cancellation are available.
    """

    limits = validate_motion_limits(motion_limits)
    joint_names = tuple(limits["joint_order"])
    if requested.joint_names != joint_names:
        raise TrajectoryGenerationError(
            "requested trajectory joints do not match "
            "motion_limits.joint_order"
        )

    positions = _finite_vector(
        measured_positions, len(joint_names), "measured_positions"
    )
    velocities = _finite_vector(
        measured_velocities, len(joint_names), "measured_velocities"
    )

    maximum_start_velocity = float(
        limits["start_state"]["max_abs_velocity"]
    )
    for joint_name, position, velocity in zip(
        joint_names, positions, velocities, strict=True
    ):
        position_limits = limits["joints"][joint_name][
            "operational_position"
        ]
        lower = float(position_limits["lower"])
        upper = float(position_limits["upper"])
        if not lower <= position <= upper:
            raise TrajectoryGenerationError(
                f"measured {joint_name} position {position:.6f} is outside "
                f"operational range [{lower:.6f}, {upper:.6f}]"
            )
        if abs(velocity) > maximum_start_velocity:
            raise TrajectoryGenerationError(
                f"measured {joint_name} velocity {velocity:.6f} exceeds "
                "stopped "
                f"start threshold {maximum_start_velocity:.6f}"
            )

    initial = _stationary_point(0.0, positions)
    points = [initial]
    segments: list[GeneratedSegment] = []
    all_peaks: list[JointPeakDynamics] = []
    current = initial

    for keyframe in requested.keyframes:
        if not math.isclose(
            current.time_from_start,
            keyframe.start_time,
            rel_tol=0.0,
            abs_tol=1e-12,
        ):
            raise TrajectoryGenerationError(
                f"requested keyframe '{keyframe.pose_name}' has "
                "inconsistent timing"
            )

        arrival = _stationary_point(keyframe.arrival_time, keyframe.positions)
        segment_index = len(segments)
        transition = GeneratedSegment(
            pose_name=keyframe.pose_name,
            kind="transition",
            start=current,
            end=arrival,
        )
        peaks = _segment_peaks(
            segment_index,
            keyframe.pose_name,
            joint_names,
            current.positions,
            arrival.positions,
            transition.duration,
        )
        _validate_peak_dynamics(peaks, limits)
        segments.append(transition)
        all_peaks.extend(peaks)
        points.append(arrival)
        current = arrival

        if keyframe.hold_until > keyframe.arrival_time:
            hold_end = _stationary_point(
                keyframe.hold_until, keyframe.positions
            )
            hold = GeneratedSegment(
                pose_name=keyframe.pose_name,
                kind="hold",
                start=current,
                end=hold_end,
            )
            segments.append(hold)
            points.append(hold_end)
            current = hold_end

    return GeneratedTrajectory(
        name=requested.name,
        description=requested.description,
        joint_names=joint_names,
        points=tuple(points),
        segments=tuple(segments),
        peak_dynamics=tuple(all_peaks),
        total_duration=requested.total_duration,
    )


def sample_segment(
    segment: GeneratedSegment, elapsed: float
) -> TrajectoryPoint:
    """Evaluate one generated segment at an absolute trajectory time."""

    if not math.isfinite(elapsed):
        raise ValueError("elapsed trajectory time must be finite")

    if elapsed <= segment.start.time_from_start:
        return segment.start
    if elapsed >= segment.end.time_from_start:
        return segment.end
    if segment.kind == "hold":
        return _stationary_point(elapsed, segment.start.positions)

    duration = segment.duration
    u = (elapsed - segment.start.time_from_start) / duration
    position_scale = 10.0 * u**3 - 15.0 * u**4 + 6.0 * u**5
    velocity_scale = (30.0 * u**2 - 60.0 * u**3 + 30.0 * u**4) / duration
    acceleration_scale = (
        60.0 * u - 180.0 * u**2 + 120.0 * u**3
    ) / duration**2

    displacements = tuple(
        end - start
        for start, end in zip(
            segment.start.positions, segment.end.positions, strict=True
        )
    )
    return TrajectoryPoint(
        time_from_start=elapsed,
        positions=tuple(
            start + displacement * position_scale
            for start, displacement in zip(
                segment.start.positions, displacements, strict=True
            )
        ),
        velocities=tuple(
            displacement * velocity_scale for displacement in displacements
        ),
        accelerations=tuple(
            displacement * acceleration_scale for displacement in displacements
        ),
    )


def sample_trajectory(
    trajectory: GeneratedTrajectory, elapsed: float
) -> tuple[TrajectoryPoint, bool]:
    """Evaluate a generated trajectory and report whether its time elapsed."""

    if not math.isfinite(elapsed):
        raise ValueError("elapsed trajectory time must be finite")
    if elapsed <= 0.0:
        return trajectory.points[0], False
    if elapsed >= trajectory.total_duration:
        return trajectory.points[-1], True

    for segment in trajectory.segments:
        if elapsed <= segment.end.time_from_start:
            return sample_segment(segment, elapsed), False

    raise RuntimeError("generated trajectory has no segment for elapsed time")
