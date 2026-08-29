"""Accept Orion's mechanically supported shoulder-rest calibration endpoint."""

from __future__ import annotations

import argparse
import json
from collections.abc import Sequence
from pathlib import Path

from .archived.motion_test import motion_test_plan, read_motion_preflight
from .bus import create_lerobot_bus
from .calibration import (
    SUPPORTED_REST_MINIMUM_JOINT,
    CalibrationError,
    accept_supported_rest_minimum,
    write_calibration_file,
)


DEFAULT_CALIBRATION = Path("~/.config/orion/servo_calibration.json")


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Capture the torque-off shoulder position resting on Orion's base "
            "and make that documented endpoint commandable."
        )
    )
    parser.add_argument("--port", required=True, help="Servo adapter serial port.")
    parser.add_argument(
        "--calibration",
        type=Path,
        default=DEFAULT_CALIBRATION,
        help=f"Calibration JSON path (default: {DEFAULT_CALIBRATION}).",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the operation without opening hardware or changing calibration.",
    )
    return parser


def _load_document(path: Path) -> dict[str, object]:
    path = path.expanduser()
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise CalibrationError(
            f"Could not read calibration '{path}': {error}"
        ) from error
    if not isinstance(document, dict):
        raise CalibrationError("Calibration root must be a mapping.")
    return document


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    calibration_path = args.calibration.expanduser()
    print("Orion supported shoulder-rest calibration:")
    print(f"  Joint: {SUPPORTED_REST_MINIMUM_JOINT}")
    print(f"  Calibration: {calibration_path}")
    print("  Servo EEPROM: unchanged")
    if args.dry_run:
        print("Dry run only: hardware was not opened and calibration was not changed.")
        return 0

    bus = None
    try:
        document = _load_document(calibration_path)
        assignments = motion_test_plan()
        bus = create_lerobot_bus(args.port, assignments)
        bus.connect(handshake=True)
        read_motion_preflight(bus, assignments)
        positions = {
            name: int(value)
            for name, value in bus.sync_read(
                "Present_Position", normalize=False, num_retry=5
            ).items()
        }
        raw_position = positions[SUPPORTED_REST_MINIMUM_JOINT]
        updated = accept_supported_rest_minimum(
            document,
            raw_position=raw_position,
        )
        shoulder = updated["joints"][  # type: ignore[index]
            SUPPORTED_REST_MINIMUM_JOINT
        ]
        delta = int(shoulder["supported_rest_minimum_delta_raw"])
        radians = delta * 2.0 * 3.141592653589793 / 4096.0
        print(
            f"\nCaptured supported rest: raw={raw_position}, "
            f"delta={delta:+d}, angle={radians:+.6f} rad"
        )
        confirmation = input(
            "Type ACCEPT SHOULDER REST to update the software calibration: "
        )
        if confirmation.strip() != "ACCEPT SHOULDER REST":
            print("Cancelled. Calibration was not changed.")
            return 2

        read_motion_preflight(bus, assignments)
        backup = write_calibration_file(updated, calibration_path)
    except (
        CalibrationError,
        ConnectionError,
        OSError,
        RuntimeError,
        ValueError,
    ) as error:
        print(f"\nSupported-rest calibration stopped: {error}")
        print("Calibration was not changed.")
        return 1
    finally:
        if bus is not None and getattr(bus, "is_connected", False):
            try:
                bus.disable_torque(num_retry=2)
            finally:
                bus.disconnect(disable_torque=True)

    print(f"\nUpdated: {calibration_path}")
    if backup is not None:
        print(f"Previous calibration backed up to: {backup}")
    print(
        "The shoulder supported-rest endpoint is now commandable; "
        "servo EEPROM is unchanged."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
