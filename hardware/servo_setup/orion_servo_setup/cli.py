"""Command-line entry point for Orion STS3215 ID provisioning."""

from __future__ import annotations

import argparse
from collections.abc import Sequence

from .bus import create_lerobot_bus
from .provisioning import (
    ORION_SERVO_ASSIGNMENTS,
    ProvisioningCancelled,
    ServoAssignment,
    provision_servos,
    provisioning_plan,
)


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Assign Orion's persistent IDs to five STS3215 servos, one servo at a time."
    )
    parser.add_argument("--port", required=True, help="Servo adapter serial port, for example /dev/ttyACM0")
    parser.add_argument(
        "--joint",
        choices=[assignment.joint_name for assignment in ORION_SERVO_ASSIGNMENTS],
        help="Program only one joint, useful for retry or replacement.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the ID plan without importing LeRobot, opening the port, or writing a servo.",
    )
    return parser


def _print_plan(plan: Sequence[ServoAssignment]) -> None:
    print("Orion STS3215 provisioning plan (EEPROM ID and common baud rate):")
    for assignment in plan:
        print(
            f"  ID {assignment.servo_id}: {assignment.joint_name} "
            f"(joint_ref_name: {assignment.joint_ref_name})"
        )


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    plan = provisioning_plan(selected_joint=args.joint)
    _print_plan(plan)

    if args.dry_run:
        print("Dry run only: no serial port was opened and no servo was changed.")
        return 0

    print(
        "\nWARNING: This writes persistent configuration to each servo. "
        "Never connect more than one unconfigured servo during a step."
    )
    bus = None
    try:
        bus = create_lerobot_bus(args.port, ORION_SERVO_ASSIGNMENTS)
        provision_servos(bus, plan)
    except ProvisioningCancelled as exc:
        print(exc)
        return 2
    except (ConnectionError, OSError, RuntimeError) as exc:
        print(f"Servo setup failed: {exc}")
        return 1
    finally:
        # setup_motor() disables torque before writing EEPROM.  Do not ask
        # disconnect() to address all five configured IDs because only the
        # current single servo is physically attached during provisioning.
        if bus is not None and getattr(bus, "is_connected", False):
            bus.disconnect(disable_torque=False)

    print("\nAll requested servo IDs were programmed. Keep the servos labelled and disconnected for now.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
