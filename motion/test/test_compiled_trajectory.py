"""Contract tests for Python consumption of the single Rust compiler."""

from __future__ import annotations

from copy import deepcopy

import pytest

from orion_motion.compiled_trajectory import (
    TrajectoryCompilerError,
    compile_trajectory,
    sample_trajectory,
    trajectory_from_document,
)


def test_rust_compiler_exports_calibrated_fixed_rate_v2_samples():
    trajectory = compile_trajectory("look_at_left_expressive", "attentive")

    assert trajectory.name == "look_at_left_expressive"
    assert trajectory.space == "absolute"
    assert trajectory.control_rate_hz == 50.0
    assert trajectory.peak_velocity_rad_s <= 5.445427266222309 * 1.001
    assert trajectory.points[0].time_from_start == 0.0
    assert trajectory.points[-1].time_from_start == trajectory.total_duration
    assert trajectory.points[-1].velocities == (0.0,) * 5
    assert trajectory.points[-1].accelerations == (0.0,) * 5
    assert tuple(marker.name for marker in trajectory.markers) == ("notice", "settled")


def test_sampler_holds_the_same_50_hz_command_between_ticks():
    trajectory = compile_trajectory("look_at_left_expressive", "attentive")

    before, before_index = sample_trajectory(trajectory, 0.019)
    tick, tick_index = sample_trajectory(trajectory, 0.020)

    assert before_index == 0
    assert before == trajectory.points[0]
    assert tick_index == 1
    assert tick == trajectory.points[1]


def test_reader_rejects_non_v2_or_non_rust_documents():
    with pytest.raises(TrajectoryCompilerError, match="format_version 2"):
        trajectory_from_document({"format_version": 1, "compiler": "python"})


def test_relative_motion_returns_exactly_to_anchor():
    trajectory = compile_trajectory("idle_breathe", "attentive")

    assert trajectory.space == "anchor_relative"
    assert trajectory.points[-1].positions == trajectory.points[0].positions
    assert 0.0 <= trajectory.amplitude_scale <= 1.0


def test_v2_compiler_uses_the_shared_rest_frame_profile():
    from orion_motion.compiled_trajectory import PROJECT_ROOT
    from orion_motion.calibration import load_calibrated_joint_ranges
    trajectory = compile_trajectory('return_home', 'rest', hardware='v2')
    profile = load_calibrated_joint_ranges(PROJECT_ROOT / 'hardware/profiles/v2-joints.json')
    assert trajectory.points[0].positions == (0.0,) * 5
    expected = {r.name: (r.lower_rad, r.upper_rad) for r in profile}
    for joint in trajectory.joint_ranges:
        assert (joint.lower_rad, joint.upper_rad) == pytest.approx(expected[joint.name])
