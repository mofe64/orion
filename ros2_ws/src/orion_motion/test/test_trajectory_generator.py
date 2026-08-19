"""Tests for shared smooth trajectory generation and dynamic validation."""

from copy import deepcopy
from pathlib import Path

import pytest

from orion_motion.motion_loader import load_yaml_file
from orion_motion.trajectory_builder import build_trajectory
from orion_motion.trajectory_generator import (
    TrajectoryGenerationError,
    generate_trajectory,
    sample_segment,
    sample_trajectory,
)


PACKAGE_DIRECTORY = Path(__file__).parent.parent
CONFIG_DIRECTORY = PACKAGE_DIRECTORY / "config"
MOTIONS_DIRECTORY = PACKAGE_DIRECTORY / "motions"


@pytest.fixture
def project_limits():
    return load_yaml_file(CONFIG_DIRECTORY / "motion_limits.yaml")


@pytest.fixture
def project_poses():
    return load_yaml_file(CONFIG_DIRECTORY / "poses.yaml")


def pose_positions(poses, pose_name, joint_names):
    return tuple(
        poses["poses"][pose_name]["positions"][joint_name]
        for joint_name in joint_names
    )


def load_requested(relative_path, poses, limits):
    return build_trajectory(
        load_yaml_file(MOTIONS_DIRECTORY / relative_path),
        poses,
        limits,
    )


def test_generates_explicit_measured_start_arrival_and_hold_points(
    project_poses, project_limits
):
    requested = load_requested(
        "functional/return_home.yaml", project_poses, project_limits
    )
    start = pose_positions(project_poses, "attentive", requested.joint_names)

    generated = generate_trajectory(
        requested,
        start,
        (0.0,) * len(start),
        project_limits,
    )

    assert generated.joint_names == requested.joint_names
    actual_times = [point.time_from_start for point in generated.points]
    assert actual_times == pytest.approx([0.0, 2.0, 2.5])
    assert generated.points[0].positions == start
    assert generated.points[0].velocities == (0.0,) * 5
    assert generated.points[0].accelerations == (0.0,) * 5
    assert [segment.kind for segment in generated.segments] == [
        "transition",
        "hold",
    ]


def test_quintic_midpoint_is_smooth_and_halfway_between_poses(
    project_poses, project_limits
):
    requested = load_requested(
        "functional/return_home.yaml", project_poses, project_limits
    )
    start = pose_positions(project_poses, "attentive", requested.joint_names)
    generated = generate_trajectory(
        requested, start, (0.0,) * 5, project_limits
    )

    midpoint, completed = sample_trajectory(generated, 1.0)
    final = generated.points[1]

    assert completed is False
    assert midpoint.positions == pytest.approx(
        tuple(
            (begin + end) / 2.0
            for begin, end in zip(start, final.positions)
        )
    )
    assert midpoint.velocities[1] == pytest.approx(
        (final.positions[1] - start[1]) * 1.875 / 2.0
    )
    assert midpoint.accelerations == pytest.approx((0.0,) * 5, abs=1e-12)


def test_transition_boundaries_match_position_velocity_and_acceleration(
    project_poses, project_limits
):
    requested = load_requested(
        "functional/return_home.yaml", project_poses, project_limits
    )
    start = pose_positions(project_poses, "attentive", requested.joint_names)
    generated = generate_trajectory(
        requested, start, (0.0,) * 5, project_limits
    )
    transition, hold = generated.segments

    assert sample_segment(transition, 0.0) == transition.start
    assert sample_segment(transition, 2.0) == transition.end
    assert transition.end == hold.start
    assert transition.end.velocities == (0.0,) * 5
    assert transition.end.accelerations == (0.0,) * 5


def test_hold_is_constant_and_reports_completion_at_total_duration(
    project_poses, project_limits
):
    requested = load_requested(
        "functional/return_home.yaml", project_poses, project_limits
    )
    start = pose_positions(project_poses, "attentive", requested.joint_names)
    generated = generate_trajectory(
        requested, start, (0.0,) * 5, project_limits
    )

    during_hold, completed_during = sample_trajectory(generated, 2.25)
    at_end, completed_at_end = sample_trajectory(generated, 2.5)

    assert completed_during is False
    assert during_hold.positions == generated.points[-1].positions
    assert during_hold.velocities == (0.0,) * 5
    assert during_hold.accelerations == (0.0,) * 5
    assert completed_at_end is True
    assert at_end == generated.points[-1]


def test_analytic_peak_dynamics_are_recorded(project_poses, project_limits):
    requested = load_requested(
        "functional/return_home.yaml", project_poses, project_limits
    )
    start = pose_positions(project_poses, "attentive", requested.joint_names)
    generated = generate_trajectory(
        requested, start, (0.0,) * 5, project_limits
    )

    shoulder = next(
        peak
        for peak in generated.peak_dynamics
        if peak.joint_name == "shoulder_pitch_joint"
    )
    assert shoulder.velocity == pytest.approx(1.875 * 0.4 / 2.0)
    assert shoulder.acceleration == pytest.approx(
        (10.0 / 3.0**0.5) * 0.4 / 2.0**2
    )
    assert shoulder.jerk == pytest.approx(60.0 * 0.4 / 2.0**3)


def test_aggressive_authored_motion_is_rejected(project_poses, project_limits):
    requested = load_requested(
        "expressive/acknowledge_expressive.yaml",
        project_poses,
        project_limits,
    )
    start = pose_positions(project_poses, "attentive", requested.joint_names)

    with pytest.raises(
        TrajectoryGenerationError,
        match="head_pitch_joint max_(velocity|acceleration|jerk)",
    ):
        generate_trajectory(requested, start, (0.0,) * 5, project_limits)


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
def test_functional_motion_library_passes_provisional_limits(
    motion_name, project_poses, project_limits
):
    requested = load_requested(
        f"functional/{motion_name}.yaml", project_poses, project_limits
    )
    start = pose_positions(project_poses, "attentive", requested.joint_names)

    generated = generate_trajectory(
        requested, start, (0.0,) * 5, project_limits
    )

    assert generated.name == motion_name


def test_measured_moving_start_is_rejected(project_poses, project_limits):
    requested = load_requested(
        "functional/return_home.yaml", project_poses, project_limits
    )
    start = pose_positions(project_poses, "attentive", requested.joint_names)

    with pytest.raises(
        TrajectoryGenerationError, match="stopped start threshold"
    ):
        generate_trajectory(
            requested,
            start,
            (0.0, 0.0, 0.06, 0.0, 0.0),
            project_limits,
        )


def test_measured_start_outside_operational_range_is_rejected(
    project_poses, project_limits
):
    requested = load_requested(
        "functional/return_home.yaml", project_poses, project_limits
    )
    start = list(
        pose_positions(project_poses, "attentive", requested.joint_names)
    )
    start[0] = 2.0

    with pytest.raises(
        TrajectoryGenerationError, match="outside operational range"
    ):
        generate_trajectory(requested, start, (0.0,) * 5, project_limits)


@pytest.mark.parametrize(
    "positions,velocities,message",
    [
        ((0.0,) * 4, (0.0,) * 5, "measured_positions"),
        ((0.0,) * 5, (0.0,) * 4, "measured_velocities"),
        ((0.0, 0.0, float("nan"), 0.0, 0.0), (0.0,) * 5, "finite"),
    ],
)
def test_measured_state_must_be_complete_and_finite(
    positions, velocities, message, project_poses, project_limits
):
    requested = load_requested(
        "functional/return_home.yaml", project_poses, project_limits
    )

    with pytest.raises(TrajectoryGenerationError, match=message):
        generate_trajectory(requested, positions, velocities, project_limits)


def test_generation_does_not_mutate_inputs(project_poses, project_limits):
    requested = load_requested(
        "functional/return_home.yaml", project_poses, project_limits
    )
    limits_before = deepcopy(project_limits)
    start = pose_positions(project_poses, "attentive", requested.joint_names)

    generate_trajectory(requested, start, (0.0,) * 5, project_limits)

    assert project_limits == limits_before
