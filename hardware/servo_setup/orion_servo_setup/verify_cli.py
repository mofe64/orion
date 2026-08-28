"""Command-line entry point for Orion's read-only STS3215 register audit."""

from __future__ import annotations

import argparse
from collections.abc import Sequence

from .bus import create_lerobot_bus
from .provisioning import ORION_SERVO_ASSIGNMENTS
from .verification import (
    REGISTER_GROUPS,
    ServoRegisterSnapshot,
    read_servo_registers,
    verification_plan,
)


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Read Orion STS3215 configuration and telemetry without writing registers."
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
        help="List the selected bus IDs without opening the serial port.",
    )
    return parser


def _print_live_summary(snapshots: Sequence[ServoRegisterSnapshot]) -> None:
    print("Live state:")
    print(
        "ID  joint                     model  fw     baud  position  velocity  load  "
        "current  voltage  temp  status  moving  torque"
    )
    for snapshot in snapshots:
        assignment = snapshot.assignment
        torque = "ON" if snapshot.torque_enabled else "off"
        print(
            f"{assignment.servo_id:<3} "
            f"{assignment.joint_name:<25} "
            f"{snapshot.model_number:<6} "
            f"{snapshot.firmware_version:<6} "
            f"{snapshot.raw('Baud_Rate'):>4}  "
            f"{snapshot.position_raw:>8}  "
            f"{snapshot.raw('Present_Velocity'):>8}  "
            f"{snapshot.raw('Present_Load'):>4}  "
            f"{snapshot.current_ma:>6.1f}mA  "
            f"{snapshot.voltage_v:>5.1f}V  "
            f"{snapshot.temperature_c:>3}C  "
            f"{snapshot.raw('Status'):>6}  "
            f"{snapshot.raw('Moving'):>6}  "
            f"{torque}"
        )


def _print_register_matrix(snapshots: Sequence[ServoRegisterSnapshot]) -> None:
    id_columns = "".join(
        f"ID {snapshot.assignment.servo_id:>2}  " for snapshot in snapshots
    )
    print("\nRegister values (raw):")
    print(f"{'register':<38}{id_columns}")
    for group, registers in REGISTER_GROUPS:
        if group in {"identity", "live telemetry"}:
            continue
        print(f"[{group}]")
        for register in registers:
            values = "".join(f"{snapshot.raw(register):>5}  " for snapshot in snapshots)
            print(f"{register:<38}{values}")


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    plan = verification_plan(selected_joint=args.joint)

    if args.dry_run:
        ids = ", ".join(str(assignment.servo_id) for assignment in plan)
        print(f"Would audit STS3215 bus IDs {ids} on {args.port}.")
        return 0

    bus = None
    try:
        bus = create_lerobot_bus(args.port, plan)
        bus.connect(handshake=False)
        snapshots = read_servo_registers(bus, plan)
    except (ConnectionError, OSError, RuntimeError, ValueError) as exc:
        print(f"Register audit failed: {exc}")
        return 1
    finally:
        # Verification is strictly read-only. Disabling torque during disconnect
        # would itself be a register write, so close the port without doing so.
        if bus is not None and getattr(bus, "is_connected", False):
            bus.disconnect(disable_torque=False)

    print(f"Orion STS3215 audit: {args.port}\n")
    _print_live_summary(snapshots)
    _print_register_matrix(snapshots)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
