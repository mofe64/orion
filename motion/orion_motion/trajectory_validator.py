"""Validate trajectories before an Orion execution backend sees them."""

from __future__ import annotations

import math
from dataclasses import InitVar, dataclass
from typing import Any, Sequence

from orion_motion.motion_validator import (
    MotionValidationError,
    validate_motion_limits,
)
from orion_motion.trajectory_generator import (
    GeneratedSegment,
    GeneratedTrajectory,
    TrajectoryPoint,
    calculate_segment_peaks,
)


_TOLERANCE = 1e-12
_DYNAMIC_LIMITS = (
    ("velocity", "max_velocity"),
    ("acceleration", "max_acceleration"),
    ("jerk", "max_jerk"),
)
_VALIDATION_TOKEN = object()


@dataclass(frozen=True)
class ValidationIssue:
    """One independently actionable trajectory defect."""

    code: str
    message: str
    segment_index: int | None = None
    pose_name: str | None = None
    joint_name: str | None = None
    measured_value: float | None = None
    limit_value: float | None = None
    minimum_duration: float | None = None
    region_name: str | None = None


@dataclass(frozen=True)
class SegmentDurationRequirement:
    """Minimum authored duration needed to clear all dynamics for a segment."""

    segment_index: int
    pose_name: str
    requested_duration: float
    minimum_duration: float


@dataclass(frozen=True)
class ValidationReport:
    """Complete, non-mutating result of validating one generated trajectory."""

    motion_name: str
    issues: tuple[ValidationIssue, ...]
    duration_requirements: tuple[SegmentDurationRequirement, ...]

    @property
    def is_valid(self) -> bool:
        """Return whether the report contains no issues."""

        return not self.issues

    def summary(self) -> str:
        """Return a compact result suitable for logs and CLI errors."""

        if self.is_valid:
            return f"trajectory '{self.motion_name}' is valid"
        first = self.issues[0]
        remainder = len(self.issues) - 1
        suffix = f"; plus {remainder} more issue(s)" if remainder else ""
        return (
            f"trajectory '{self.motion_name}' has {len(self.issues)} "
            f"validation issue(s): {first.message}{suffix}"
        )


@dataclass(frozen=True)
class ValidatedTrajectory:
    """Execution capability issued only after a trajectory passes validation."""

    trajectory: GeneratedTrajectory
    report: ValidationReport
    _token: InitVar[object]

    def __post_init__(self, _token: object) -> None:
        if _token is not _VALIDATION_TOKEN or not self.report.is_valid:
            raise ValueError(
                "ValidatedTrajectory can only be issued by "
                "require_valid_trajectory()"
            )


class TrajectoryValidationError(ValueError):
    """Raised when an execution capability is requested for an invalid path."""

    def __init__(self, report: ValidationReport):
        """Preserve the complete rejection report on the exception."""

        self.report = report
        super().__init__(report.summary())


def _finite_number(value: Any) -> bool:
    return (
        not isinstance(value, bool)
        and isinstance(value, (int, float))
        and math.isfinite(value)
    )


def validate_forbidden_regions(
    data: Any, motion_limits: Any
) -> dict[str, Any]:
    """Validate axis-aligned forbidden configuration-space regions."""

    limits = validate_motion_limits(motion_limits)
    if not isinstance(data, dict):
        raise MotionValidationError("forbidden_regions must be a mapping")
    if (
        type(data.get("format_version")) is not int
        or data["format_version"] != 1
    ):
        raise MotionValidationError(
            "forbidden_regions.format_version must be the integer 1"
        )
    if data.get("units") != "radians":
        raise MotionValidationError("forbidden_regions.units must be 'radians'")

    regions = data.get("regions")
    if not isinstance(regions, list):
        raise MotionValidationError("forbidden_regions.regions must be a list")

    known_joints = set(limits["joint_order"])
    seen_names: set[str] = set()
    for region_index, region in enumerate(regions):
        path = f"forbidden_regions.regions[{region_index}]"
        if not isinstance(region, dict):
            raise MotionValidationError(f"{path} must be a mapping")
        name = region.get("name")
        if not isinstance(name, str) or not name:
            raise MotionValidationError(
                f"{path}.name must be a non-empty string"
            )
        if name in seen_names:
            raise MotionValidationError(
                f"forbidden_regions region name '{name}' is duplicated"
            )
        seen_names.add(name)
        description = region.get("description")
        if description is not None and not isinstance(description, str):
            raise MotionValidationError(f"{path}.description must be a string")

        joints = region.get("joints")
        if not isinstance(joints, dict) or not joints:
            raise MotionValidationError(
                f"{path}.joints must be a non-empty mapping"
            )
        unknown = set(joints) - known_joints
        if unknown:
            raise MotionValidationError(
                f"{path}.joints contains unknown joints: {sorted(unknown)}"
            )
        for joint_name, interval in joints.items():
            interval_path = f"{path}.joints.{joint_name}"
            if not isinstance(interval, dict):
                raise MotionValidationError(
                    f"{interval_path} must be a mapping"
                )
            lower = interval.get("lower")
            upper = interval.get("upper")
            if not _finite_number(lower) or not _finite_number(upper):
                raise MotionValidationError(
                    f"{interval_path}.lower and upper must be finite numbers"
                )
            if lower >= upper:
                raise MotionValidationError(
                    f"{interval_path}.lower must be less than upper"
                )
            mechanical = limits["joints"][joint_name]["mechanical_position"]
            if lower < mechanical["lower"] or upper > mechanical["upper"]:
                raise MotionValidationError(
                    f"{interval_path} must be contained by mechanical_position"
                )

    return data


def _append_vector_issues(
    issues: list[ValidationIssue],
    point: TrajectoryPoint,
    point_index: int,
    joint_names: tuple[str, ...],
    limits: dict[str, Any],
) -> None:
    vectors = (
        ("positions", point.positions),
        ("velocities", point.velocities),
        ("accelerations", point.accelerations),
    )
    expected = len(joint_names)
    for field_name, values in vectors:
        if not isinstance(values, Sequence) or len(values) != expected:
            issues.append(
                ValidationIssue(
                    code="INVALID_POINT_SHAPE",
                    message=(
                        f"point {point_index} {field_name} must contain "
                        f"{expected} values"
                    ),
                )
            )
            continue
        for joint_name, value in zip(joint_names, values, strict=True):
            if not _finite_number(value):
                issues.append(
                    ValidationIssue(
                        code="NONFINITE_POINT_VALUE",
                        message=(
                            f"point {point_index} {joint_name} {field_name} "
                            "must be finite"
                        ),
                        joint_name=joint_name,
                    )
                )
                continue
            if field_name == "positions":
                for range_name, code in (
                    ("mechanical_position", "MECHANICAL_POSITION_LIMIT"),
                    ("operational_position", "OPERATIONAL_POSITION_LIMIT"),
                ):
                    interval = limits["joints"][joint_name][range_name]
                    if not interval["lower"] <= value <= interval["upper"]:
                        issues.append(
                            ValidationIssue(
                                code=code,
                                message=(
                                    f"point {point_index} {joint_name} position "
                                    f"{value:.6f} is outside {range_name} "
                                    f"[{interval['lower']:.6f}, "
                                    f"{interval['upper']:.6f}]"
                                ),
                                joint_name=joint_name,
                                measured_value=float(value),
                            )
                        )
            elif field_name in {"velocities", "accelerations"}:
                limit_field = (
                    "max_velocity"
                    if field_name == "velocities"
                    else "max_acceleration"
                )
                allowed = float(limits["joints"][joint_name][limit_field])
                if abs(value) > allowed + _TOLERANCE:
                    issues.append(
                        ValidationIssue(
                            code=f"POINT_{field_name.upper()}_LIMIT",
                            message=(
                                f"point {point_index} {joint_name} "
                                f"{field_name} magnitude {abs(value):.6f} "
                                f"exceeds {limit_field} {allowed:.6f}"
                            ),
                            joint_name=joint_name,
                            measured_value=abs(float(value)),
                            limit_value=allowed,
                        )
                    )


def _segment_intersects_region(
    segment: GeneratedSegment,
    joint_indices: dict[str, int],
    region: dict[str, Any],
) -> bool:
    """Intersect a straight joint-space path with an axis-aligned region."""

    path_lower = 0.0
    path_upper = 1.0
    for joint_name, interval in region["joints"].items():
        index = joint_indices[joint_name]
        start = segment.start.positions[index]
        end = segment.end.positions[index]
        if not _finite_number(start) or not _finite_number(end):
            return False
        delta = end - start
        if abs(delta) <= _TOLERANCE:
            if not interval["lower"] <= start <= interval["upper"]:
                return False
            continue
        first = (interval["lower"] - start) / delta
        second = (interval["upper"] - start) / delta
        coordinate_lower = min(first, second)
        coordinate_upper = max(first, second)
        path_lower = max(path_lower, coordinate_lower)
        path_upper = min(path_upper, coordinate_upper)
        if path_lower > path_upper + _TOLERANCE:
            return False
    return path_lower <= 1.0 + _TOLERANCE and path_upper >= -_TOLERANCE


def _minimum_duration(displacement: float, field: str, limit: float) -> float:
    if field == "velocity":
        return 1.875 * displacement / limit
    if field == "acceleration":
        return math.sqrt((10.0 / math.sqrt(3.0)) * displacement / limit)
    return (60.0 * displacement / limit) ** (1.0 / 3.0)


def validate_trajectory(
    trajectory: GeneratedTrajectory,
    motion_limits: Any,
    forbidden_regions: Any,
) -> ValidationReport:
    """Collect every structural, positional, dynamic, and path violation."""

    limits = validate_motion_limits(motion_limits)
    regions_data = validate_forbidden_regions(forbidden_regions, limits)
    issues: list[ValidationIssue] = []
    joint_names = tuple(limits["joint_order"])

    if trajectory.joint_names != joint_names:
        issues.append(
            ValidationIssue(
                code="JOINT_ORDER_MISMATCH",
                message=(
                    "trajectory joint_names do not match "
                    "motion_limits.joint_order"
                ),
            )
        )
    if not trajectory.points:
        issues.append(
            ValidationIssue(code="NO_POINTS", message="trajectory has no points")
        )
    if not trajectory.segments:
        issues.append(
            ValidationIssue(code="NO_SEGMENTS", message="trajectory has no segments")
        )
    if (
        not _finite_number(trajectory.total_duration)
        or trajectory.total_duration <= 0
    ):
        issues.append(
            ValidationIssue(
                code="INVALID_TOTAL_DURATION",
                message=(
                    "trajectory total_duration must be finite and greater "
                    "than zero"
                ),
            )
        )

    previous_time: float | None = None
    for point_index, point in enumerate(trajectory.points):
        point_time = point.time_from_start
        if not _finite_number(point_time):
            issues.append(
                ValidationIssue(
                    code="NONFINITE_POINT_TIME",
                    message=f"point {point_index} time_from_start must be finite",
                )
            )
        elif point_time < 0 or (
            previous_time is not None and point_time <= previous_time
        ):
            issues.append(
                ValidationIssue(
                    code="NONINCREASING_POINT_TIME",
                    message=f"point {point_index} time_from_start must increase",
                )
            )
        if _finite_number(point_time):
            previous_time = float(point_time)
        _append_vector_issues(issues, point, point_index, joint_names, limits)

    duration_minima: dict[int, float] = {}
    for segment_index, segment in enumerate(trajectory.segments):
        if segment.kind not in {"transition", "hold"}:
            issues.append(
                ValidationIssue(
                    code="INVALID_SEGMENT_KIND",
                    message=(
                        f"segment {segment_index} has invalid kind "
                        f"'{segment.kind}'"
                    ),
                    segment_index=segment_index,
                    pose_name=segment.pose_name,
                )
            )
        start_time = segment.start.time_from_start
        end_time = segment.end.time_from_start
        if not _finite_number(start_time) or not _finite_number(end_time):
            issues.append(
                ValidationIssue(
                    code="INVALID_SEGMENT_TIME",
                    message=(
                        f"segment {segment_index} endpoint times must be "
                        "finite"
                    ),
                    segment_index=segment_index,
                    pose_name=segment.pose_name,
                )
            )
            continue
        duration = float(end_time) - float(start_time)
        if duration <= 0:
            issues.append(
                ValidationIssue(
                    code="INVALID_SEGMENT_DURATION",
                    message=(
                        f"segment {segment_index} duration must be positive "
                        "and finite"
                    ),
                    segment_index=segment_index,
                    pose_name=segment.pose_name,
                )
            )
            continue
        start_positions = segment.start.positions
        end_positions = segment.end.positions
        endpoint_vectors = (
            start_positions,
            segment.start.velocities,
            segment.start.accelerations,
            end_positions,
            segment.end.velocities,
            segment.end.accelerations,
        )
        if any(
            not isinstance(vector, Sequence)
            or len(vector) != len(joint_names)
            for vector in endpoint_vectors
        ):
            issues.append(
                ValidationIssue(
                    code="INVALID_SEGMENT_SHAPE",
                    message=(
                        f"segment {segment_index} endpoints must contain "
                        "all joints"
                    ),
                    segment_index=segment_index,
                    pose_name=segment.pose_name,
                )
            )
            continue
        if (
            segment_index
            and segment.start != trajectory.segments[segment_index - 1].end
        ):
            issues.append(
                ValidationIssue(
                    code="DISCONTINUOUS_SEGMENTS",
                    message=(
                        f"segment {segment_index} does not start at the "
                        "previous endpoint"
                    ),
                    segment_index=segment_index,
                    pose_name=segment.pose_name,
                )
            )
        if (
            segment.kind == "hold"
            and segment.start.positions != segment.end.positions
        ):
            issues.append(
                ValidationIssue(
                    code="MOVING_HOLD",
                    message=f"hold segment {segment_index} changes position",
                    segment_index=segment_index,
                    pose_name=segment.pose_name,
                )
            )
        if segment.kind == "hold" and any(
            abs(value) > _TOLERANCE
            for vector in (
                segment.start.velocities,
                segment.start.accelerations,
                segment.end.velocities,
                segment.end.accelerations,
            )
            for value in vector
            if _finite_number(value)
        ):
            issues.append(
                ValidationIssue(
                    code="MOVING_HOLD_BOUNDARY",
                    message=(
                        f"hold segment {segment_index} must have zero "
                        "velocity and acceleration"
                    ),
                    segment_index=segment_index,
                    pose_name=segment.pose_name,
                )
            )

        if segment.kind == "transition" and all(
            _finite_number(value)
            for positions in (start_positions, end_positions)
            for value in positions
        ):
            peaks = calculate_segment_peaks(
                segment_index,
                segment.pose_name,
                joint_names,
                segment.start.positions,
                segment.end.positions,
                duration,
            )
            for peak in peaks:
                displacement = abs(
                    segment.end.positions[joint_names.index(peak.joint_name)]
                    - segment.start.positions[joint_names.index(peak.joint_name)]
                )
                for field, limit_field in _DYNAMIC_LIMITS:
                    measured = getattr(peak, field)
                    allowed = float(
                        limits["joints"][peak.joint_name][limit_field]
                    )
                    if measured > allowed + _TOLERANCE:
                        minimum = _minimum_duration(displacement, field, allowed)
                        duration_minima[segment_index] = max(
                            duration_minima.get(segment_index, 0.0), minimum
                        )
                        issues.append(
                            ValidationIssue(
                                code=f"{field.upper()}_LIMIT",
                                message=(
                                    f"segment {segment_index} to pose "
                                    f"'{segment.pose_name}' exceeds "
                                    f"{peak.joint_name} "
                                    f"{limit_field}: peak {measured:.6f}, "
                                    f"limit {allowed:.6f}"
                                ),
                                segment_index=segment_index,
                                pose_name=segment.pose_name,
                                joint_name=peak.joint_name,
                                measured_value=measured,
                                limit_value=allowed,
                                minimum_duration=minimum,
                            )
                        )

    if trajectory.segments and trajectory.points:
        expected_points = (trajectory.segments[0].start,) + tuple(
            segment.end for segment in trajectory.segments
        )
        if trajectory.points != expected_points:
            issues.append(
                ValidationIssue(
                    code="POINT_SEGMENT_MISMATCH",
                    message=(
                        "trajectory points do not exactly describe the "
                        "segment boundaries"
                    ),
                )
            )
        if trajectory.segments[0].start != trajectory.points[0]:
            issues.append(
                ValidationIssue(
                    code="START_POINT_MISMATCH",
                    message="first segment does not begin at the first point",
                )
            )
        if trajectory.segments[-1].end != trajectory.points[-1]:
            issues.append(
                ValidationIssue(
                    code="END_POINT_MISMATCH",
                    message="last segment does not end at the last point",
                )
            )
        final_time = trajectory.points[-1].time_from_start
        duration_matches = (
            _finite_number(final_time)
            and _finite_number(trajectory.total_duration)
            and math.isclose(
                final_time,
                trajectory.total_duration,
                rel_tol=0.0,
                abs_tol=_TOLERANCE,
            )
        )
        if not duration_matches:
            issues.append(
                ValidationIssue(
                    code="TOTAL_DURATION_MISMATCH",
                    message="last point time does not match total_duration",
                )
            )

    if trajectory.joint_names == joint_names:
        indices = {name: index for index, name in enumerate(joint_names)}
        for segment_index, segment in enumerate(trajectory.segments):
            if len(segment.start.positions) != len(joint_names) or len(
                segment.end.positions
            ) != len(joint_names):
                continue
            for region in regions_data["regions"]:
                if _segment_intersects_region(segment, indices, region):
                    issues.append(
                        ValidationIssue(
                            code="FORBIDDEN_REGION",
                            message=(
                                f"segment {segment_index} to pose "
                                f"'{segment.pose_name}' intersects forbidden "
                                f"region '{region['name']}'"
                            ),
                            segment_index=segment_index,
                            pose_name=segment.pose_name,
                            region_name=region["name"],
                        )
                    )

    requirements = tuple(
        SegmentDurationRequirement(
            segment_index=index,
            pose_name=trajectory.segments[index].pose_name,
            requested_duration=trajectory.segments[index].duration,
            minimum_duration=minimum,
        )
        for index, minimum in sorted(duration_minima.items())
    )
    return ValidationReport(
        motion_name=trajectory.name,
        issues=tuple(issues),
        duration_requirements=requirements,
    )


def require_valid_trajectory(
    trajectory: GeneratedTrajectory,
    motion_limits: Any,
    forbidden_regions: Any,
) -> ValidatedTrajectory:
    """Return an execution capability or raise with the complete report."""

    report = validate_trajectory(trajectory, motion_limits, forbidden_regions)
    if not report.is_valid:
        raise TrajectoryValidationError(report)
    return ValidatedTrajectory(
        trajectory=trajectory,
        report=report,
        _token=_VALIDATION_TOKEN,
    )
