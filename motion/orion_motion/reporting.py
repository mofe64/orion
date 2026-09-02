"""Create and compare machine-readable Orion motion run reports."""

from __future__ import annotations

import argparse
from dataclasses import asdict
import hashlib
import json
import math
from pathlib import Path
import platform
import sys
from typing import Any, Sequence

from orion_motion.execution_types import (
    ExecutionResult,
    execution_result_data,
)
from orion_motion.compiled_trajectory import CompiledTrajectory


REPORT_FORMAT_VERSION = 2


def file_sha256(path: Path) -> str:
    """Return the SHA-256 digest of one source or configuration file."""

    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(65536), b""):
            digest.update(block)
    return digest.hexdigest()


def _peak_dynamics_by_joint(trajectory: CompiledTrajectory) -> dict[str, Any]:
    peaks = {
        name: {"velocity": 0.0, "acceleration": 0.0}
        for name in trajectory.joint_names
    }
    for sample in trajectory.samples:
        for index, name in enumerate(trajectory.joint_names):
            joint = peaks[name]
            joint["velocity"] = max(joint["velocity"], abs(sample.velocities[index]))
            joint["acceleration"] = max(
                joint["acceleration"], abs(sample.accelerations[index])
            )
    return peaks


def build_run_report(
    *,
    motion_path: Path,
    calibration_path: Path,
    trajectory: CompiledTrajectory,
    start_positions: Sequence[float],
    start_velocities: Sequence[float],
    start_state_age: float,
    result: ExecutionResult,
) -> dict[str, Any]:
    """Build one self-contained execution report for later comparison."""

    return {
        "format_version": REPORT_FORMAT_VERSION,
        "motion_source": {
            "path": str(motion_path),
            "sha256": file_sha256(motion_path),
        },
        "calibration_source": {
            "path": str(calibration_path),
            "sha256": file_sha256(calibration_path),
        },
        "environment": {
            "python": platform.python_version(),
            "platform": platform.platform(),
        },
        "measured_start": {
            "joint_names": list(trajectory.joint_names),
            "positions": list(start_positions),
            "velocities": list(start_velocities),
            "age_seconds": start_state_age,
        },
        "trajectory": {
            "name": trajectory.name,
            "description": trajectory.description,
            "compiler_revision": trajectory.build_revision,
            "space": trajectory.space,
            "style": trajectory.style,
            "joint_names": list(trajectory.joint_names),
            "total_duration": trajectory.total_duration,
            "control_rate_hz": trajectory.control_rate_hz,
            "peak_velocity_rad_s": trajectory.peak_velocity_rad_s,
            "amplitude_scale": trajectory.amplitude_scale,
            "points": [asdict(point) for point in trajectory.points],
            "markers": [asdict(marker) for marker in trajectory.markers],
            "peak_desired_by_joint": _peak_dynamics_by_joint(trajectory),
        },
        "execution": execution_result_data(result),
    }


def write_json_report(path: Path, data: dict[str, Any]) -> None:
    """Write one report atomically enough for a local validation run."""

    resolved = path.resolve()
    resolved.parent.mkdir(parents=True, exist_ok=True)
    resolved.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")


def load_run_report(path: Path) -> dict[str, Any]:
    """Load one versioned Orion run report."""

    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ValueError(f"run report '{path}' must contain a JSON object")
    if data.get("format_version") != REPORT_FORMAT_VERSION:
        raise ValueError(
            f"run report '{path}' format_version must be "
            f"{REPORT_FORMAT_VERSION}"
        )
    return data


def _same_number(first: Any, second: Any) -> bool:
    return (
        isinstance(first, (int, float))
        and not isinstance(first, bool)
        and isinstance(second, (int, float))
        and not isinstance(second, bool)
        and math.isclose(float(first), float(second), rel_tol=0.0, abs_tol=1e-9)
    )


def _trajectory_contract(report: dict[str, Any]) -> dict[str, Any]:
    """Return the exact Rust-compiled command stream shared by backends."""

    trajectory = report["trajectory"]
    return {
        "space": trajectory["space"],
        "style": trajectory["style"],
        "control_rate_hz": trajectory["control_rate_hz"],
        "markers": trajectory["markers"],
        "points": trajectory["points"],
    }


def _peak_differences(
    first: dict[str, Any], second: dict[str, Any]
) -> dict[str, dict[str, float]]:
    first_peaks = first["trajectory"]["peak_desired_by_joint"]
    second_peaks = second["trajectory"]["peak_desired_by_joint"]
    return {
        joint_name: {
            field: abs(
                float(first_peaks[joint_name][field])
                - float(second_peaks[joint_name][field])
            )
            for field in ("velocity", "acceleration")
        }
        for joint_name in first["trajectory"]["joint_names"]
    }


def compare_run_reports(
    first: dict[str, Any], second: dict[str, Any]
) -> dict[str, Any]:
    """Compare shared intent and measured outcomes from two backends."""

    issues: list[str] = []
    first_trajectory = first["trajectory"]
    second_trajectory = second["trajectory"]
    first_execution = first["execution"]
    second_execution = second["execution"]

    if first_execution["backend"] == second_execution["backend"]:
        issues.append("reports use the same backend label")
    for label, first_value, second_value in (
        ("motion name", first_trajectory["name"], second_trajectory["name"]),
        (
            "motion source hash",
            first["motion_source"]["sha256"],
            second["motion_source"]["sha256"],
        ),
        (
            "calibration hash",
            first["calibration_source"]["sha256"],
            second["calibration_source"]["sha256"],
        ),
        (
            "joint order",
            first_trajectory["joint_names"],
            second_trajectory["joint_names"],
        ),
        (
            "compiled sample stream",
            _trajectory_contract(first),
            _trajectory_contract(second),
        ),
    ):
        if first_value != second_value:
            issues.append(f"{label} differs")
    if not _same_number(
        first_trajectory["total_duration"],
        second_trajectory["total_duration"],
    ):
        issues.append("generated duration differs")
    for report in (first, second):
        execution = report["execution"]
        if execution["status"] != "succeeded":
            issues.append(
                f"backend {execution['backend']} ended with "
                f"status {execution['status']}"
            )

    return {
        "format_version": REPORT_FORMAT_VERSION,
        "motion_name": first_trajectory["name"],
        "backends": [
            first_execution["backend"],
            second_execution["backend"],
        ],
        "passed": not issues,
        "issues": issues,
        "shared_contract": {
            "motion_sha256": first["motion_source"]["sha256"],
            "calibration_sha256": first["calibration_source"]["sha256"],
            "joint_names": first_trajectory["joint_names"],
            "total_duration": first_trajectory["total_duration"],
            "compiled_trajectory": _trajectory_contract(first),
        },
        "desired_peak_differences_by_joint": _peak_differences(first, second),
        "measured_outcomes": {
            first_execution["backend"]: {
                "status": first_execution["status"],
                "metrics": first_execution["metrics"],
                "peak_desired_by_joint": first_trajectory[
                    "peak_desired_by_joint"
                ],
            },
            second_execution["backend"]: {
                "status": second_execution["status"],
                "metrics": second_execution["metrics"],
                "peak_desired_by_joint": second_trajectory[
                    "peak_desired_by_joint"
                ],
            },
        },
    }


def parse_compare_arguments(arguments: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Compare two Orion motion execution reports."
    )
    parser.add_argument("first", type=Path, help="First run-report JSON file.")
    parser.add_argument("second", type=Path, help="Second run-report JSON file.")
    parser.add_argument(
        "--output",
        type=Path,
        help="Optional path for the machine-readable comparison JSON.",
    )
    return parser.parse_args(arguments)


def run_compare(arguments: Sequence[str] | None = None) -> int:
    options = parse_compare_arguments(
        list(arguments) if arguments is not None else sys.argv[1:]
    )
    comparison = compare_run_reports(
        load_run_report(options.first),
        load_run_report(options.second),
    )
    print(f"Motion: {comparison['motion_name']}")
    print(f"Backends: {', '.join(comparison['backends'])}")
    print(f"Parity result: {'passed' if comparison['passed'] else 'failed'}")
    for issue in comparison["issues"]:
        print(f"  - {issue}")
    if options.output is not None:
        write_json_report(options.output, comparison)
        print(f"Machine-readable comparison: {options.output.resolve()}")
    return 0 if comparison["passed"] else 1


def main() -> None:
    raise SystemExit(run_compare())
