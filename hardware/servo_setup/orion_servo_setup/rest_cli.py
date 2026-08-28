"""CLI for capturing Orion's mechanically stable torque-free rest pose."""

from __future__ import annotations

import argparse
import time
from collections.abc import Sequence
from pathlib import Path

from .bus import create_lerobot_bus
from .motion_test import motion_test_plan, read_motion_preflight
from .pose_execution import load_hardware_calibration
from .rest_capture import (
    STABILITY_DURATION_SECONDS,
    RestCaptureError,
    load_operational_ranges,
    positions_to_rest_angles,
    validate_rest_stability,
    write_rest_pose,
)


DEFAULT_CALIBRATION = Path("~/.config/orion/servo_calibration.json")
ORION_ROOT = Path(__file__).resolve().parents[3]
DEFAULT_POSES = ORION_ROOT / "ros2_ws" / "src" / "orion_motion" / "config" / "poses.yaml"
DEFAULT_LIMITS = (
    ORION_ROOT / "ros2_ws" / "src" / "orion_motion" / "config" / "motion_limits.yaml"
)
SAMPLE_INTERVAL_SECONDS = 0.10


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Capture a torque-off, mechanically stable Orion rest pose and add it to poses.yaml."
        )
    )
    parser.add_argument("--port", required=True, help="Servo adapter serial port.")
    parser.add_argument("--calibration", type=Path, default=DEFAULT_CALIBRATION)
    parser.add_argument("--poses", type=Path, default=DEFAULT_POSES)
    parser.add_argument("--limits", type=Path, default=DEFAULT_LIMITS)
    parser.add_argument(
        "--replace",
        action="store_true",
        help="Replace an existing rest pose after another full stability check.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the capture plan without opening hardware or changing poses.yaml.",
    )
    return parser


def _positions(bus) -> dict[str, int]:
    return {
        name: int(value)
        for name, value in bus.sync_read(
            "Present_Position", normalize=False, num_retry=5
        ).items()
    }


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    print("Orion torque-free rest-pose capture:")
    print(f"  Stability observation: {STABILITY_DURATION_SECONDS:.0f} seconds")
    print("  Maximum permitted drift: 10 raw steps (~0.88 deg)")
    print(f"  Pose library: {args.poses}")
    print("  Servo EEPROM: unchanged")
    if args.dry_run:
        print("Dry run only: no serial port was opened and poses.yaml was not changed.")
        return 0

    try:
        calibration = load_hardware_calibration(args.calibration)
        operational_ranges = load_operational_ranges(args.limits)
    except (OSError, RuntimeError, ValueError) as exc:
        print(f"Cannot prepare rest capture: {exc}")
        return 1

    confirmation = input(
        "\nPlace Orion over a clear padded area. Turn 6 V ON, keep all supports in place, "
        "and type CAPTURE REST to connect with torque off: "
    )
    if confirmation.strip() != "CAPTURE REST":
        print("Rest capture cancelled. No serial port was opened and no file was changed.")
        return 2

    bus = None
    try:
        assignments = motion_test_plan()
        bus = create_lerobot_bus(args.port, assignments)
        bus.connect(handshake=True)
        read_motion_preflight(bus, assignments)
        print("\nPreflight passed: all five servos are healthy and torque is off.")
        input(
            "Slowly arrange a low, balanced pose that supports itself. Remove every block and "
            "keep hands completely clear. Wait for it to settle, then press ENTER to start the "
            "five-second torque-off stability check: "
        )

        reference = _positions(bus)
        samples: list[dict[str, int]] = []
        deadline = time.monotonic() + STABILITY_DURATION_SECONDS
        while time.monotonic() < deadline:
            time.sleep(SAMPLE_INTERVAL_SECONDS)
            samples.append(_positions(bus))
        maximum_drift = validate_rest_stability(reference, samples)
        angles = positions_to_rest_angles(reference, calibration, operational_ranges)

        print("\nCandidate rest pose passed the torque-off stability and range checks:")
        print("  joint                         raw   radians   max_drift_raw")
        for assignment in assignments:
            name = assignment.joint_name
            print(
                f"  {name:<29} {reference[name]:5d} "
                f"{angles[name]:+9.5f} {maximum_drift[name]:15d}"
            )
        save_confirmation = input(
            "Keep hands clear. Type SAVE REST to write this pose into poses.yaml: "
        )
        if save_confirmation.strip() != "SAVE REST":
            print("Rest capture cancelled. poses.yaml was not changed.")
            return 2

        final_positions = _positions(bus)
        validate_rest_stability(reference, [final_positions])
        read_motion_preflight(bus, assignments)
        write_rest_pose(args.poses, angles, replace=args.replace)
    except KeyboardInterrupt:
        print("\nRest capture interrupted. Torque remains off; poses.yaml was not changed.")
        return 130
    except (ConnectionError, OSError, RuntimeError, ValueError) as exc:
        print(f"\nRest capture stopped: {exc}")
        print("Torque remains off. No new rest pose was accepted.")
        return 1
    finally:
        if bus is not None and getattr(bus, "is_connected", False):
            try:
                bus.disable_torque(num_retry=2)
            finally:
                bus.disconnect(disable_torque=True)

    print(f"\nRest pose saved to: {args.poses}")
    print("All torque is off. The lamp should remain still; turn 6 V OFF now.")
    print("If it moves at all, restore power over padding and capture a lower rest pose.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
