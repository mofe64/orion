"""Tests for Orion's execution-gating trajectory validator."""

from dataclasses import replace
from pathlib import Path

import pytest

from orion_motion.motion_loader import load_yaml_file
from orion_motion.motion_validator import MotionValidationError
from orion_motion.trajectory_builder import build_trajectory
from orion_motion.trajectory_generator import generate_trajectory
from orion_motion.trajectory_validator import (
    TrajectoryValidationError,
    ValidatedTrajectory,
    require_valid_trajectory,
    validate_forbidden_regions,
    validate_trajectory,
)


PACKAGE_DIRECTORY = Path(__file__).parent.parent
CONFIG_DIRECTORY = PACKAGE_DIRECTORY / "config"
MOTIONS_DIRECTORY = PACKAGE_DIRECTORY / "motions"


@pytest.fixture
def project_data():
    return (
        load_yaml_file(CONFIG_DIRECTORY / "poses.yaml"),
        load_yaml_file(CONFIG_DIRECTORY / "motion_limits.yaml"),
        load_yaml_file(CONFIG_DIRECTORY / "forbidden_regions.yaml"),
    )


def generate(relative_motion_path, project_data):
    poses, limits, _ = project_data
    requested = build_trajectory(
        load_yaml_file(MOTIONS_DIRECTORY / relative_motion_path),
        poses,
        limits,
    )
    start = tuple(
        poses["poses"]["attentive"]["positions"][joint_name]
        for joint_name in requested.joint_names
    )
    return generate_trajectory(requested, start, (0.0,) * 5, limits)


@pytest.mark.parametrize(
    "motion_name",
    [
        "acknowledge",
        "look_at_left",
        "look_at_right",
        "return_home",
        "target_unreachable",
    ],
)
def test_functional_motions_receive_execution_capability(
    motion_name, project_data
):
    _, limits, regions = project_data
    generated = generate(f"functional/{motion_name}.yaml", project_data)

    validated = require_valid_trajectory(generated, limits, regions)

    assert validated.trajectory is generated
    assert validated.report.is_valid
    assert validated.report.issues == ()


@pytest.mark.parametrize(
    "motion_name",
    [
        "acknowledge_expressive",
        "look_at_left_expressive",
        "target_unreachable_expressive",
    ],
)
def test_expressive_motions_receive_execution_capability(
    motion_name, project_data
):
    _, limits, regions = project_data
    generated = generate(f"expressive/{motion_name}.yaml", project_data)

    validated = require_valid_trajectory(generated, limits, regions)

    assert validated.trajectory is generated
    assert validated.report.is_valid
    assert validated.report.issues == ()


def test_rejection_carries_complete_report_and_does_not_retime(project_data):
    poses, limits, regions = project_data
    motion = load_yaml_file(
        MOTIONS_DIRECTORY / "expressive/acknowledge_expressive.yaml"
    )
    motion["motion"]["keyframes"][1]["duration"] = 0.25
    motion["motion"]["keyframes"][2]["duration"] = 0.20
    motion["motion"]["keyframes"][3]["duration"] = 0.30
    requested = build_trajectory(motion, poses, limits)
    start = tuple(
        poses["poses"]["attentive"]["positions"][joint_name]
        for joint_name in requested.joint_names
    )
    generated = generate_trajectory(requested, start, (0.0,) * 5, limits)
    authored_times = tuple(point.time_from_start for point in generated.points)

    with pytest.raises(TrajectoryValidationError) as caught:
        require_valid_trajectory(generated, limits, regions)

    assert len(caught.value.report.issues) == 9
    assert "plus 8 more issue(s)" in str(caught.value)
    assert tuple(point.time_from_start for point in generated.points) == authored_times


def test_execution_capability_cannot_be_constructed_directly(project_data):
    _, limits, regions = project_data
    generated = generate("functional/look_at_left.yaml", project_data)
    report = validate_trajectory(generated, limits, regions)

    with pytest.raises(ValueError, match="require_valid_trajectory"):
        ValidatedTrajectory(generated, report, object())


def test_continuous_path_detects_region_between_safe_endpoints(project_data):
    _, limits, _ = project_data
    generated = generate("functional/look_at_left.yaml", project_data)
    regions = {
        "format_version": 1,
        "units": "radians",
        "regions": [
            {
                "name": "base_crossing_example",
                "description": "Synthetic validator regression fixture.",
                "joints": {
                    "base_yaw_joint": {"lower": -0.70, "upper": -0.60}
                },
            }
        ],
    }

    report = validate_trajectory(generated, limits, regions)

    assert generated.segments[0].start.positions[0] == pytest.approx(-0.30)
    assert generated.segments[0].end.positions[0] == pytest.approx(-1.00)
    crossings = [
        issue for issue in report.issues if issue.code == "FORBIDDEN_REGION"
    ]
    assert len(crossings) == 1
    assert crossings[0].region_name == "base_crossing_example"


def test_project_forbidden_region_contract_is_explicitly_empty(project_data):
    _, limits, regions = project_data

    validated = validate_forbidden_regions(regions, limits)

    assert validated["regions"] == []


@pytest.mark.parametrize(
    "region, message",
    [
        (
            {
                "name": "unknown_joint",
                "joints": {"missing_joint": {"lower": -0.2, "upper": 0.2}},
            },
            "unknown joints",
        ),
        (
            {
                "name": "reversed_interval",
                "joints": {
                    "base_yaw_joint": {"lower": 0.2, "upper": -0.2}
                },
            },
            "lower must be less than upper",
        ),
    ],
)
def test_forbidden_region_configuration_rejects_invalid_contract(
    region, message, project_data
):
    _, limits, _ = project_data
    data = {"format_version": 1, "units": "radians", "regions": [region]}

    with pytest.raises(MotionValidationError, match=message):
        validate_forbidden_regions(data, limits)


def test_structural_and_position_defects_are_collected_without_crashing(
    project_data,
):
    _, limits, regions = project_data
    generated = generate("functional/return_home.yaml", project_data)
    bad_first = replace(
        generated.points[0],
        time_from_start=float("nan"),
        positions=(99.0,) + generated.points[0].positions[1:],
        velocities=generated.points[0].velocities[:-1],
    )
    malformed = replace(
        generated,
        points=(bad_first,) + generated.points[1:],
        joint_names=tuple(reversed(generated.joint_names)),
    )

    report = validate_trajectory(malformed, limits, regions)
    codes = {issue.code for issue in report.issues}

    assert "JOINT_ORDER_MISMATCH" in codes
    assert "NONFINITE_POINT_TIME" in codes
    assert "INVALID_POINT_SHAPE" in codes
    assert "MECHANICAL_POSITION_LIMIT" in codes
    assert "OPERATIONAL_POSITION_LIMIT" in codes
    assert "START_POINT_MISMATCH" in codes


def test_non_numeric_segment_time_is_reported_without_crashing(project_data):
    _, limits, regions = project_data
    generated = generate("functional/return_home.yaml", project_data)
    first_segment = generated.segments[0]
    malformed_end = replace(first_segment.end, time_from_start="not-a-time")
    malformed_segment = replace(first_segment, end=malformed_end)
    malformed = replace(
        generated,
        segments=(malformed_segment,) + generated.segments[1:],
    )

    report = validate_trajectory(malformed, limits, regions)

    assert "INVALID_SEGMENT_TIME" in {
        issue.code for issue in report.issues
    }
