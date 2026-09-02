"""Tests for durable motion-run reports and backend comparison."""

from copy import deepcopy
from pathlib import Path

from orion_motion.execution_types import (
    ExecutionResult,
    ExecutionStatus,
)
from orion_motion.compiled_trajectory import compile_trajectory
from orion_motion.reporting import build_run_report, compare_run_reports


PACKAGE_DIRECTORY = Path(__file__).parent.parent
CONFIG_DIRECTORY = PACKAGE_DIRECTORY / "config"
MOTION_PATH = PACKAGE_DIRECTORY / "motions/functional/look_at_left.yaml"
CALIBRATION_PATH = (
    PACKAGE_DIRECTORY.parent / "simulation/mujoco/config/servo_calibration.json"
)


def make_report(backend):
    trajectory = compile_trajectory(
        "look_at_left",
        "attentive",
        pose_file=CONFIG_DIRECTORY / "poses.yaml",
        motions_directory=PACKAGE_DIRECTORY / "motions",
        calibration_file=CALIBRATION_PATH,
    )
    start = trajectory.points[0].positions
    result = ExecutionResult(
        motion_name=trajectory.name,
        backend=backend,
        status=ExecutionStatus.SUCCEEDED,
        message="done",
    )
    return build_run_report(
        motion_path=MOTION_PATH,
        calibration_path=CALIBRATION_PATH,
        trajectory=trajectory,
        start_positions=start,
        start_velocities=(0.0,) * 5,
        start_state_age=0.01,
        result=result,
    )


def test_report_contains_sources_generated_path_and_measured_start():
    report = make_report("native_test")

    assert report["format_version"] == 2
    assert len(report["motion_source"]["sha256"]) == 64
    assert len(report["calibration_source"]["sha256"]) == 64
    assert report["trajectory"]["name"] == "look_at_left"
    assert report["trajectory"]["points"]
    assert report["trajectory"]["control_rate_hz"] == 50.0
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
