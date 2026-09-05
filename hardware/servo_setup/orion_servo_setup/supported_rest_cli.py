"""Accept a mechanically supported Orion rest calibration endpoint."""

from __future__ import annotations

import argparse
import json
import math
from collections.abc import Sequence
from pathlib import Path

from .preflight import commissioning_plan, read_preflight
from .bus import create_lerobot_bus
from .calibration import (
    ENCODER_RESOLUTION,
    SUPPORTED_REST_JOINTS,
    CalibrationError,
    accept_supported_rest_endpoint,
    circular_delta,
    write_calibration_file,
)


DEFAULT_CALIBRATION = Path("~/.config/orion/servo_calibration.json")


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Allow a live, torque-off supported rest endpoint."
    )
    parser.add_argument("--port", required=True, help="Servo adapter serial port.")
    parser.add_argument(
        "--joint",
        choices=sorted(SUPPORTED_REST_JOINTS),
        default="shoulder_pitch_joint",
        help="Supported joint endpoint to accept (default: shoulder_pitch_joint).",
    )
    parser.add_argument(
        "--calibration",
        type=Path,
        default=DEFAULT_CALIBRATION,
        help=f"Calibration JSON path (default: {DEFAULT_CALIBRATION}).",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Show the selected port and calibration without opening hardware.",
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
    if args.dry_run:
        print(
            f"Would read {args.joint} supported rest on {args.port}; "
            f"calibration: {calibration_path}"
        )
        return 0

    print(f"Orion supported rest: {args.port} {args.joint}")
    bus = None
    try:
        document = _load_document(calibration_path)
        assignments = commissioning_plan()
        bus = create_lerobot_bus(args.port, assignments)
        bus.connect(handshake=True)
        read_preflight(bus, assignments)
        positions = {
            name: int(value)
            for name, value in bus.sync_read(
                "Present_Position", normalize=False, num_retry=5
            ).items()
        }
        raw_position = positions[args.joint]
        joints = document.get("joints")
        current = joints.get(args.joint) if isinstance(joints, dict) else None
        if not isinstance(current, dict):
            raise CalibrationError(f"Calibration is missing {args.joint}.")
        neutral = current.get("neutral_raw")
        safe_min = current.get("safe_min_delta_raw")
        safe_max = current.get("safe_max_delta_raw")
        if type(neutral) is not int or type(safe_min) is not int or type(safe_max) is not int:
            raise CalibrationError(f"{args.joint} calibration limits must be integers.")
        delta = circular_delta(raw_position, neutral)
        radians = delta * 2.0 * math.pi / ENCODER_RESOLUTION
        if safe_min <= delta <= safe_max:
            print(
                f"Already allowed: raw={raw_position} delta={delta:+d} "
                f"angle={radians:+.6f} rad"
            )
            return 0

        endpoint = "minimum" if delta < safe_min else "maximum"
        updated = accept_supported_rest_endpoint(
            document,
            joint_name=args.joint,
            raw_position=raw_position,
        )
        joint = updated["joints"][args.joint]  # type: ignore[index]
        delta = int(joint[f"supported_rest_{endpoint}_delta_raw"])
        radians = delta * 2.0 * math.pi / ENCODER_RESOLUTION
        print(
            f"Candidate: raw={raw_position} delta={delta:+d} "
            f"angle={radians:+.6f} rad"
        )
        confirmation = input("Save? [y/N] ")
        if confirmation.strip().lower() != "y":
            print("Cancelled.")
            return 2

        read_preflight(bus, assignments)
        backup = write_calibration_file(updated, calibration_path)
    except (
        CalibrationError,
        ConnectionError,
        OSError,
        RuntimeError,
        ValueError,
    ) as error:
        print(f"Supported rest failed: {error}")
        return 1
    finally:
        if bus is not None and getattr(bus, "is_connected", False):
            try:
                bus.disable_torque(num_retry=2)
            finally:
                bus.disconnect(disable_torque=True)

    print(f"Saved: {calibration_path}")
    if backup is not None:
        print(f"Backup: {backup}")
    print("Torque: off")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
