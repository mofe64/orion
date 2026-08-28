"""Command-line entry point for read-only Orion servo verification."""

from __future__ import annotations

import argparse
from collections.abc import Sequence

from .bus import create_lerobot_bus
from .provisioning import ORION_SERVO_ASSIGNMENTS, ServoAssignment
from .verification import ServoTelemetry, read_servo_telemetry, verification_plan


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Read and report Orion STS3215 identities and telemetry without commanding movement."
    )
    parser.add_argument("--port", required=True, help="Servo adapter serial port.")
    parser.add_argument(
        "--joint",
        choices=[assignment.joint_name for assignment in ORION_SERVO_ASSIGNMENTS],
        help="Verify only one joint instead of the complete five-servo bus.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the verification plan without importing LeRobot or opening the serial port.",
    )
    return parser


def _print_plan(plan: Sequence[ServoAssignment]) -> None:
    print("Orion STS3215 read-only verification plan:")
    for assignment in plan:
        print(
            f"  ID {assignment.servo_id}: {assignment.joint_name} "
            f"(joint_ref_name: {assignment.joint_ref_name})"
        )


def _print_telemetry(snapshots: Sequence[ServoTelemetry]) -> None:
    print("\nVerification results:")
    print("  ID  joint                     model  position_raw  voltage  temp  torque")
    for snapshot in snapshots:
        assignment = snapshot.assignment
        torque = "ON" if snapshot.torque_enabled else "off"
        print(
            f"  {assignment.servo_id:<3} "
            f"{assignment.joint_name:<25} "
            f"{snapshot.model_number:<6} "
            f"{snapshot.position_raw:<13} "
            f"{snapshot.voltage_v:>4.1f} V  "
            f"{snapshot.temperature_c:>3} C  "
            f"{torque}"
        )


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    plan = verification_plan(selected_joint=args.joint)
    _print_plan(plan)

    if args.dry_run:
        print("Dry run only: no serial port was opened and no servo register was read or changed.")
        return 0

    confirmation = input(
        "\nTurn servo power OFF, connect only the labelled servo(s) listed above, and keep horns "
        "unloaded. Turn servo power ON, then type VERIFY to perform read-only checks: "
    )
    if confirmation.strip() != "VERIFY":
        print("Verification cancelled. No serial port was opened.")
        return 2

    bus = None
    try:
        bus = create_lerobot_bus(args.port, plan)
        bus.connect(handshake=True)
        snapshots = read_servo_telemetry(bus, plan)
    except (ConnectionError, OSError, RuntimeError, ValueError) as exc:
        print(f"Servo verification failed: {exc}")
        return 1
    finally:
        # Verification is strictly read-only. Disabling torque during disconnect
        # would itself be a register write, so close the port without doing so.
        if bus is not None and getattr(bus, "is_connected", False):
            bus.disconnect(disable_torque=False)

    _print_telemetry(snapshots)
    print("\nRead-only verification passed. No servo configuration or movement command was written.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
