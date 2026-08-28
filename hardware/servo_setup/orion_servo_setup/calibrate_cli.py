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
    SAFE_MARGIN_RAW,
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
from .motion_test import motion_test_plan, read_motion_preflight


DEFAULT_OUTPUT_PATH = Path("~/.config/orion/servo_calibration.json")


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Capture neutral and safe ranges for all five assembled Orion joints in one "
            "torque-off session. No servo EEPROM register is changed."
        )
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
        help="Print the plan without importing LeRobot, opening the port, or writing a file.",
    )
    return parser


def _print_plan() -> None:
    print("Orion one-session five-joint calibration plan:")
    for assignment in motion_test_plan():
        protection = (
            " (limit to about +/-90 deg from neutral)"
            if assignment.servo_id in (1, 4)
            else ""
        )
        print(f"  ID {assignment.servo_id}: {assignment.joint_name}{protection}")
    print("Torque stays off. The command captures one neutral pose and one combined range sweep.")
    print(
        "No homing offset, limit, direction, torque, or goal register is written to servo EEPROM."
    )


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
            f"ID{capture.assignment.servo_id} "
            f"{capture.measured_min_delta_raw:+d}/{capture.measured_max_delta_raw:+d}"
        )
    return "  ".join(fields)


def _record_until_enter(bus, neutral_positions: Mapping[str, int]) -> dict[str, JointRangeCapture]:
    captures = initialize_captures(neutral_positions)
    print("Recording now. Press ENTER only after every joint has been swept.")
    last_display = 0.0
    while True:
        positions = bus.sync_read("Present_Position", normalize=False, num_retry=5)
        captures = update_captures(captures, positions)
        now = time.monotonic()
        if now - last_display >= 0.25:
            print(f"\r{_format_capture_line(captures):<110}", end="", flush=True)
            last_display = now
        if _enter_pressed():
            print()
            return captures
        time.sleep(0.05)


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    plan = motion_test_plan()
    _print_plan()

    if args.dry_run:
        print("Dry run only: no serial port was opened and no calibration file was written.")
        return 0

    confirmation = input(
        "\nKeep the 6 V supply OFF while arranging padded supports under the arm and head. "
        "Keep the cutoff within reach. Turn 6 V ON; if the lamp remains still, type "
        "CALIBRATE ALL: "
    )
    if confirmation.strip() != "CALIBRATE ALL":
        print("Calibration cancelled. No serial port was opened and no file was written.")
        return 2

    bus = None
    try:
        bus = create_lerobot_bus(args.port, plan)
        bus.connect(handshake=True)
        read_motion_preflight(bus, plan)
        print("\nPreflight passed: all five STS3215 servos are healthy and torque is off.")

        input(
            "With the links still supported, place Orion in the reference LeLamp zero/middle "
            "pose. Do not force a joint. Press ENTER to capture neutral: "
        )
        neutral_positions = {
            name: int(value)
            for name, value in bus.sync_read(
                "Present_Position", normalize=False, num_retry=5
            ).items()
        }
        print("Neutral captured:")
        for assignment in plan:
            print(f"  ID {assignment.servo_id}: raw {neutral_positions[assignment.joint_name]}")

        input(
            "\nRange sweep: move ONE joint at a time slowly through its usable travel, keeping "
            "the other links supported. Stop before collision, cable tension, or a hard stop. "
            "For IDs 1 and 4, go only about 90 degrees each way from neutral. Do not force the "
            "gearbox. Press ENTER to start recording: "
        )
        captures = _record_until_enter(bus, neutral_positions)
        validate_captures(captures)
        for joint_name in sorted(YAW_JOINTS):
            capture = captures[joint_name]
            if (
                capture.measured_min_delta_raw < -YAW_SAFE_LIMIT_RAW
                or capture.measured_max_delta_raw > YAW_SAFE_LIMIT_RAW
            ):
                print(
                    f"WARNING: {joint_name} was swept beyond the protected yaw range; "
                    f"its commandable limits will be capped at +/-{YAW_SAFE_LIMIT_RAW} raw "
                    "steps (~88.2 deg)."
                )

        # Repeat the same safety checks after handling the mechanism and before
        # accepting its measurements.
        read_motion_preflight(bus, plan)
        document = build_calibration_document(captures, port=args.port)
        backup = write_calibration_file(document, args.output)
    except KeyboardInterrupt:
        print("\nCalibration aborted. No new calibration was accepted; disabling all torque.")
        return 130
    except (CalibrationError, ConnectionError, OSError, RuntimeError, ValueError) as exc:
        print(f"\nCalibration stopped: {exc}")
        print("No new calibration was accepted. Disabling all torque.")
        return 1
    finally:
        if bus is not None and getattr(bus, "is_connected", False):
            try:
                bus.disable_torque(num_retry=2)
            finally:
                bus.disconnect(disable_torque=True)

    print("\nCalibration capture complete for all five joints.")
    print(f"Saved: {args.output.expanduser()}")
    if backup is not None:
        print(f"Previous calibration backed up to: {backup}")
    print(
        f"Measured endpoints were reduced inward by {SAFE_MARGIN_RAW} raw steps "
        "(~1.76 deg) for the software safety limits."
    )
    print("Servo EEPROM was not changed. All five servos have torque disabled; turn 6 V OFF.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
