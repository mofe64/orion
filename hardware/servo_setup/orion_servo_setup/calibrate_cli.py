"""One-session calibration capture for all five assembled Orion joints."""

from __future__ import annotations

import argparse
import select
import sys
import time
from collections.abc import Mapping, Sequence
from pathlib import Path

from .bus import create_lerobot_bus
from .calibration import (
    CalibrationError,
    JointRangeCapture,
    YAW_JOINTS,
    YAW_SAFE_LIMIT_RAW,
    build_calibration_document,
    initialize_captures,
    update_captures,
    validate_captures,
    write_calibration_file,
)
from .preflight import commissioning_plan, read_preflight


DEFAULT_OUTPUT_PATH = Path("~/.config/orion/servo_calibration.json")


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Capture Orion's torque-off zero and joint ranges."
    )
    parser.add_argument("--port", required=True, help="Servo adapter serial port.")
    parser.add_argument(
        "--output",
        type=Path,
        default=DEFAULT_OUTPUT_PATH,
        help=f"Calibration JSON path (default: {DEFAULT_OUTPUT_PATH}).",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Show the selected bus and output without opening hardware.",
    )
    return parser


def _enter_pressed() -> bool:
    ready, _, _ = select.select([sys.stdin], [], [], 0)
    if not ready:
        return False
    sys.stdin.readline()
    return True


def _format_capture_line(captures: Mapping[str, JointRangeCapture]) -> str:
    fields = []
    for capture in sorted(captures.values(), key=lambda item: item.assignment.servo_id):
        fields.append(
            f"{capture.assignment.servo_id}:"
            f"{capture.measured_min_delta_raw:+d}/{capture.measured_max_delta_raw:+d}"
        )
    return " ".join(fields)


def _record_until_enter(bus, neutral_positions: Mapping[str, int]) -> dict[str, JointRangeCapture]:
    captures = initialize_captures(neutral_positions)
    print("Recording: sweep every joint, then press ENTER.")
    last_display = 0.0
    while True:
        positions = bus.sync_read("Present_Position", normalize=False, num_retry=5)
        captures = update_captures(captures, positions)
        now = time.monotonic()
        if now - last_display >= 0.25:
            print(f"\r{_format_capture_line(captures)}", end="", flush=True)
            last_display = now
        if _enter_pressed():
            print()
            return captures
        time.sleep(0.05)


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    plan = commissioning_plan()

    if args.dry_run:
        ids = ",".join(str(item.servo_id) for item in plan)
        print(
            f"Would calibrate IDs {ids} on {args.port}; "
            f"output: {args.output.expanduser()}"
        )
        return 0

    print(f"Orion STS3215 calibration: {args.port}")
    bus = None
    try:
        bus = create_lerobot_bus(args.port, plan)
        bus.connect(handshake=True)
        read_preflight(bus, plan)
        print("Preflight: 5 servos healthy, torque off.")

        input("Set the zero pose, then press ENTER: ")
        neutral_positions = {
            name: int(value)
            for name, value in bus.sync_read(
                "Present_Position", normalize=False, num_retry=5
            ).items()
        }
        zero_values = " ".join(
            f"{item.servo_id}:{neutral_positions[item.joint_name]}" for item in plan
        )
        print(f"Zero: {zero_values}")

        input(
            "Press ENTER to record all ranges (IDs 1 and 4: about +/-90 deg): "
        )
        captures = _record_until_enter(bus, neutral_positions)
        validate_captures(captures)
        capped_joints: list[str] = []
        for joint_name in sorted(YAW_JOINTS):
            capture = captures[joint_name]
            if (
                capture.measured_min_delta_raw < -YAW_SAFE_LIMIT_RAW
                or capture.measured_max_delta_raw > YAW_SAFE_LIMIT_RAW
            ):
                capped_joints.append(joint_name)
        if capped_joints:
            print(
                f"Capped at +/-{YAW_SAFE_LIMIT_RAW}: " + ", ".join(capped_joints)
            )

        # Repeat the same safety checks after handling the mechanism and before
        # accepting its measurements.
        read_preflight(bus, plan)
        document = build_calibration_document(captures, port=args.port)
        backup = write_calibration_file(document, args.output)
    except KeyboardInterrupt:
        print("\nCalibration cancelled; torque off.")
        return 130
    except (CalibrationError, ConnectionError, OSError, RuntimeError, ValueError) as exc:
        print(f"\nCalibration failed: {exc}")
        return 1
    finally:
        if bus is not None and getattr(bus, "is_connected", False):
            try:
                bus.disable_torque(num_retry=2)
            finally:
                bus.disconnect(disable_torque=True)

    print(f"Saved: {args.output.expanduser()}")
    if backup is not None:
        print(f"Backup: {backup}")
    print("Torque: off")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
