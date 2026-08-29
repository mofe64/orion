"""Tests for durable motion-run reports and backend comparison."""

from copy import deepcopy
from pathlib import Path

from orion_motion.execution_types import (
    ExecutionResult,
    ExecutionStatus,
)
from orion_motion.motion_loader import load_yaml_file
from orion_motion.reporting import build_run_report, compare_run_reports
from orion_motion.trajectory_builder import build_trajectory
from orion_motion.trajectory_generator import generate_trajectory
from orion_motion.trajectory_validator import require_valid_trajectory


PACKAGE_DIRECTORY = Path(__file__).parent.parent
CONFIG_DIRECTORY = PACKAGE_DIRECTORY / "config"
MOTION_PATH = PACKAGE_DIRECTORY / "motions/functional/look_at_left.yaml"


def make_report(backend):
    poses = load_yaml_file(CONFIG_DIRECTORY / "poses.yaml")
    limits_path = CONFIG_DIRECTORY / "motion_limits.yaml"
    limits = load_yaml_file(limits_path)
    requested = build_trajectory(
        load_yaml_file(MOTION_PATH),
        poses,
        limits,
    )
    start = tuple(
        poses["poses"]["attentive"]["positions"][joint_name]
        for joint_name in requested.joint_names
    )
    generated = generate_trajectory(requested, start, (0.0,) * 5, limits)
    validated = require_valid_trajectory(
        generated,
        limits,
        load_yaml_file(CONFIG_DIRECTORY / "forbidden_regions.yaml"),
    )
    result = ExecutionResult(
        motion_name=requested.name,
        backend=backend,
        status=ExecutionStatus.SUCCEEDED,
        message="done",
    )
    return build_run_report(
        motion_path=MOTION_PATH,
        limits_path=limits_path,
        validated=validated,
        start_positions=start,
        start_velocities=(0.0,) * 5,
        start_state_age=0.01,
        result=result,
    )


def test_report_contains_sources_generated_path_and_measured_start():
    report = make_report("native_test")

    assert report["format_version"] == 1
    assert len(report["motion_source"]["sha256"]) == 64
    assert len(report["limits_source"]["sha256"]) == 64
    assert report["trajectory"]["name"] == "look_at_left"
    assert report["trajectory"]["segments"]
    assert report["trajectory"]["peak_desired_by_joint"]
    assert report["measured_start"]["age_seconds"] == 0.01
    assert report["execution"]["backend"] == "native_test"
    assert report["environment"]["python"]
    assert report["environment"]["platform"]


def test_matching_backends_pass_shared_contract_comparison():
    hardware = make_report("native_hardware")
    mujoco = make_report("native_mujoco")

    comparison = compare_run_reports(hardware, mujoco)

    assert comparison["passed"]
    assert comparison["issues"] == []


def test_source_or_outcome_mismatch_fails_comparison():
    hardware = make_report("native_hardware")
    mujoco = deepcopy(make_report("native_mujoco"))
    mujoco["motion_source"]["sha256"] = "different"
    mujoco["execution"]["status"] = "failed"

    comparison = compare_run_reports(hardware, mujoco)

    assert not comparison["passed"]
    assert "motion source hash differs" in comparison["issues"]
    assert any("ended with status failed" in issue for issue in comparison["issues"])
