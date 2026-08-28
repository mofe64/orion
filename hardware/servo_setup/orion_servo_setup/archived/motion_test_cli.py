"""CLI for Orion's conservative, sequential five-joint first-motion test."""

from __future__ import annotations

import argparse
from collections.abc import Sequence

from ..bus import create_lerobot_bus
from .motion_test import (
    ACCELERATION_RAW,
    GOAL_VELOCITY_RAW,
    NUDGE_STEPS,
    TORQUE_LIMIT_RAW,
    MotionTestError,
    motion_test_plan,
    nudge_joint,
    read_motion_preflight,
)


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Test all five assembled Orion joints in one session while torquing and nudging "
            "only one joint at a time."
        )
    )
    parser.add_argument("--port", required=True, help="Servo adapter serial port.")
    parser.add_argument(
        "--start-id",
        type=int,
        choices=range(1, 6),
        default=1,
        help="Begin prompts at this servo ID while still preflighting and disabling all five.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the motion plan without importing LeRobot, opening the port, or writing a register.",
    )
    return parser


def _print_plan(start_id: int) -> None:
    print("Orion sequential five-joint first-motion plan:")
    for assignment in motion_test_plan():
        action = "test" if assignment.servo_id >= start_id else "preflight only"
        print(f"  ID {assignment.servo_id}: {assignment.joint_name} ({action})")
    print(
        f"Limits: {NUDGE_STEPS} raw steps (~0.88 deg), velocity {GOAL_VELOCITY_RAW}, "
        f"acceleration {ACCELERATION_RAW}, torque {TORQUE_LIMIT_RAW}/1000."
    )
    print("Only one joint is enabled at a time; every joint is disabled after its nudge.")


def _direction_prompt(servo_id: int, joint_name: str, position_raw: int) -> int | None:
    response = input(
        f"\nID {servo_id} {joint_name} is at raw {position_raw}. Check that the joint has visible "
        "clearance and keep the 6 V cutoff within reach.\n"
        "Type 'NUDGE +' or 'NUDGE -' to test about 0.88 degrees, SKIP to leave it off, "
        "or ABORT to stop: "
    ).strip().upper()
    if response == "NUDGE +":
        return 1
    if response == "NUDGE -":
        return -1
    if response == "SKIP":
        return None
    if response == "ABORT":
        raise KeyboardInterrupt
    raise MotionTestError("Expected 'NUDGE +', 'NUDGE -', SKIP, or ABORT.")


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    plan = motion_test_plan()
    test_plan = tuple(item for item in plan if item.servo_id >= args.start_id)
    _print_plan(args.start_id)

    if args.dry_run:
        print("Dry run only: no serial port was opened and no servo register was read or written.")
        return 0

    confirmation = input(
        "\nKeep servo power OFF while arranging soft supports. Keep hands clear and the 6 V cutoff "
        "within reach. Turn servo power ON; if every joint remains still, type TEST ALL to begin "
        "the guarded session: "
    )
    if confirmation.strip() != "TEST ALL":
        print("First-motion test cancelled. No serial port was opened.")
        return 2

    bus = None
    results = []
    skipped = []
    try:
        bus = create_lerobot_bus(args.port, plan)
        bus.connect(handshake=True)
        preflight = read_motion_preflight(bus, plan)

        print("\nPreflight passed: all five STS3215 servos are in position mode with torque off.")
        for snapshot in preflight:
            assignment = snapshot.assignment
            if assignment.servo_id < args.start_id:
                continue
            direction = _direction_prompt(
                assignment.servo_id,
                assignment.joint_name,
                snapshot.position_raw,
            )
            if direction is None:
                skipped.append(assignment)
                print(f"Skipped ID {assignment.servo_id}; torque remained off.")
                continue

            result = nudge_joint(bus, assignment, direction=direction)
            results.append(result)
            response = "target reached" if result.reached_target else "directional response confirmed"
            print(
                f"ID {assignment.servo_id} passed: {result.start_position_raw} -> "
                f"{result.final_position_raw}, peak current {result.peak_current_ma:.0f} mA, "
                f"{result.final_temperature_c} C, {response}; torque is off."
            )
    except KeyboardInterrupt:
        print("\nTest aborted by operator. Disabling all torque.")
        return 130
    except (ConnectionError, OSError, RuntimeError, ValueError) as exc:
        print(f"\nFirst-motion test stopped: {exc}")
        print("All remaining joints were skipped. Disabling all torque.")
        return 1
    finally:
        if bus is not None and getattr(bus, "is_connected", False):
            try:
                bus.disable_torque(num_retry=2)
            finally:
                bus.disconnect(disable_torque=True)

    print(
        f"\nFirst-motion session complete for IDs {args.start_id}-5: "
        f"{len(results)} passed, {len(skipped)} skipped."
    )
    print("All five servos have torque disabled. Turn the 6 V servo supply off.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
