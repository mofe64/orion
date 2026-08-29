"""CLI for capturing Orion's mechanically stable torque-free rest pose."""

from __future__ import annotations

import argparse
import time
from collections.abc import Sequence
from pathlib import Path

from .bus import create_lerobot_bus
from .archived.motion_test import motion_test_plan, read_motion_preflight
from .archived.pose_execution import load_hardware_calibration
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
    if args.dry_run:
        print(f"Would capture rest on {args.port}; poses: {args.poses}")
        return 0

    print(f"Orion rest capture: {args.port}")

    try:
        calibration = load_hardware_calibration(args.calibration)
        operational_ranges = load_operational_ranges(args.limits)
    except (OSError, RuntimeError, ValueError) as exc:
        print(f"Cannot prepare rest capture: {exc}")
        return 1

    input("6 V on and supports in place. Press ENTER to connect: ")

    bus = None
    try:
        assignments = motion_test_plan()
        bus = create_lerobot_bus(args.port, assignments)
        bus.connect(handshake=True)
        read_motion_preflight(bus, assignments)
        print("Preflight: 5 servos healthy, torque off.")
        input(
            "Set stable unsupported rest, clear hands and blocks, then press ENTER: "
        )

        reference = _positions(bus)
        print(f"Checking stability ({STABILITY_DURATION_SECONDS:.0f} s)...")
        samples: list[dict[str, int]] = []
        deadline = time.monotonic() + STABILITY_DURATION_SECONDS
        while time.monotonic() < deadline:
            time.sleep(SAMPLE_INTERVAL_SECONDS)
            samples.append(_positions(bus))
        maximum_drift = validate_rest_stability(reference, samples)
        angles = positions_to_rest_angles(reference, calibration, operational_ranges)

        pose_values = " ".join(
            f"{assignment.servo_id}:{angles[assignment.joint_name]:+.5f}"
            for assignment in assignments
        )
        print(f"Pose (rad): {pose_values}")
        print(f"Max drift: {max(maximum_drift.values())} raw")
        if input("Save rest pose? [y/N] ").strip().lower() != "y":
            print("Cancelled.")
            return 2

        final_positions = _positions(bus)
        validate_rest_stability(reference, [final_positions])
        read_motion_preflight(bus, assignments)
        write_rest_pose(args.poses, angles, replace=True)
    except KeyboardInterrupt:
        print("\nCancelled; torque off.")
        return 130
    except (ConnectionError, OSError, RuntimeError, ValueError) as exc:
        print(f"\nRest capture failed: {exc}")
        print("Torque: off")
        return 1
    finally:
        if bus is not None and getattr(bus, "is_connected", False):
            try:
                bus.disable_torque(num_retry=2)
            finally:
                bus.disconnect(disable_torque=True)

    print(f"Saved: {args.poses}")
    print("Torque: off")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
