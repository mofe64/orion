"""Write persistent homing offsets so no joint range crosses raw 0/4095."""

from __future__ import annotations

import argparse
import json
from collections.abc import Sequence
from pathlib import Path

from .bus import create_lerobot_bus
from .calibration import CalibrationError, circular_delta, write_calibration_file
from .centring import CentringStep, centred_document, plan_centring, shifted_raw
from .preflight import commissioning_plan, read_preflight
from .provisioning import assignments_for_hardware

CONFIRMATION = "CENTRE"
# Torque is off and the lamp is not touched, so readings may only jitter.
POSITION_TOLERANCE_RAW = 8


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Centre each servo's raw frame on its calibrated zero (writes servo EEPROM)."
    )
    parser.add_argument("--port", required=True, help="Servo adapter serial port.")
    parser.add_argument("--hardware", choices=("v1", "v2"), default="v1")
    parser.add_argument("--calibration", type=Path, help="Calibration JSON to centre.")
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Show the planned offsets without opening hardware or changing files.",
    )
    return parser


def _print_plan(steps: Sequence[CentringStep]) -> None:
    for step in steps:
        state = "crosses 0/4095" if step.crosses_boundary else "inside 0..4095"
        print(
            f"ID {step.servo_id} {step.joint_name}: raw {step.lowest_raw}..{step.highest_raw} "
            f"({state}); offset {step.current_offset_raw} -> {step.target_offset_raw}"
        )


def _read_offsets(bus, steps: Sequence[CentringStep]) -> dict[str, int]:
    return {
        step.joint_name: int(
            bus.read("Homing_Offset", step.joint_name, normalize=False, num_retry=2)
        )
        for step in steps
    }


def _positions(bus) -> dict[str, int]:
    return {
        name: int(value)
        for name, value in bus.sync_read("Present_Position", normalize=False, num_retry=5).items()
    }


def _write_offsets(bus, steps: Sequence[CentringStep], target: dict[str, int]) -> None:
    """Write offsets with EEPROM unlocked, verify by read-back, then relock."""

    bus.disable_torque(num_retry=2)  # Also unlocks EEPROM on Feetech servos.
    try:
        for step in steps:
            bus.write(
                "Homing_Offset", step.joint_name, target[step.joint_name],
                normalize=False, num_retry=2,
            )
        observed = _read_offsets(bus, steps)
        wrong = {name: value for name, value in observed.items() if value != target[name]}
        if wrong:
            raise RuntimeError(f"Homing offset read-back did not match: {wrong}")
    finally:
        for step in steps:
            bus.write("Lock", step.joint_name, 1, normalize=False, num_retry=2)


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    if args.calibration is None:
        args.calibration = Path(
            "~/.config/orion/servo_calibration-v2.json"
            if args.hardware == "v2" else "~/.config/orion/servo_calibration.json"
        )
    path = args.calibration.expanduser()
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(document, dict) or document.get("hardware", "v1") != args.hardware:
            raise CalibrationError(f"{path} is not a {args.hardware} calibration.")
        steps = plan_centring(document)
        updated = centred_document(document, steps)
    except (OSError, UnicodeError, json.JSONDecodeError, CalibrationError) as error:
        print(f"Centring failed: {error}")
        return 1

    print(f"Orion {args.hardware} servo centring: {path}")
    _print_plan(steps)
    if not any(step.changes_offset for step in steps):
        print("Every servo is already centred; nothing to write.")
        return 0
    if args.dry_run:
        print("Dry run: no serial port opened and no files changed.")
        return 0

    plan = commissioning_plan(assignments_for_hardware(args.hardware))
    if {item.joint_name: item.servo_id for item in plan} != {
        step.joint_name: step.servo_id for step in steps
    }:
        print("Centring failed: calibration servo IDs do not match the hardware profile.")
        return 1

    bus = None
    previous: dict[str, int] | None = None
    writing = False
    target = {step.joint_name: step.target_offset_raw for step in steps}
    try:
        bus = create_lerobot_bus(args.port, plan)
        bus.connect(handshake=True)
        read_preflight(bus, plan)
        previous = _read_offsets(bus, steps)
        for step in steps:
            # The calibration must describe the servo's present frame. A servo
            # already at the target offset is a resumed, interrupted run.
            if previous[step.joint_name] not in (step.current_offset_raw, step.target_offset_raw):
                raise CalibrationError(
                    f"ID {step.servo_id} has offset {previous[step.joint_name]}, but the "
                    f"calibration expects {step.current_offset_raw}; recalibrate instead."
                )
        before = _positions(bus)
        print("Torque stays off and nothing moves. Keep the lamp still while offsets are written.")
        if input(f"Type {CONFIRMATION} to write the offsets to servo EEPROM: ").strip() != CONFIRMATION:
            print("Cancelled; nothing was written.")
            return 2

        writing = True
        _write_offsets(bus, steps, target)
        after = _positions(bus)
        for step in steps:
            expected = shifted_raw(before[step.joint_name], previous[step.joint_name], target[step.joint_name])
            if abs(circular_delta(after[step.joint_name], expected)) > POSITION_TOLERANCE_RAW:
                raise RuntimeError(
                    f"ID {step.servo_id} reads {after[step.joint_name]} after centring; expected "
                    f"about {expected}. The lamp moved or the servo applies offsets differently."
                )
        backup = write_calibration_file(updated, path)
    except KeyboardInterrupt:
        print("\nCentring interrupted.")
        return 130
    except (CalibrationError, ConnectionError, OSError, RuntimeError, ValueError) as error:
        print(f"Centring failed: {error}")
        if writing and previous is not None and getattr(bus, "is_connected", False):
            try:
                _write_offsets(bus, steps, previous)
                print("Restored the previous offsets; the calibration file is unchanged.")
            except Exception as restore_error:  # noqa: BLE001 - report both failures.
                print(
                    f"Could not restore previous offsets ({restore_error}). oriond will refuse "
                    "to start until offsets and calibration match; re-run this command."
                )
        return 1
    finally:
        if bus is not None and getattr(bus, "is_connected", False):
            try:
                bus.disable_torque(num_retry=2)
            finally:
                bus.disconnect(disable_torque=True)

    print(f"Saved: {path}")
    if backup is not None:
        print(f"Backup: {backup}")
    print("Offsets written and verified. Torque: off")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
