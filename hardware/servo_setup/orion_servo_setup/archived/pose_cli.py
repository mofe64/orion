"""CLI for a guarded physical cycle through one existing Orion named pose."""

from __future__ import annotations

import argparse
import math
import signal
from collections.abc import Sequence
from pathlib import Path

from ..bus import create_lerobot_bus
from .motion_test import motion_test_plan, read_motion_preflight
from .pose_execution import (
    MAX_POSE_TEMPERATURE_C,
    MIN_POSE_DURATION_SECONDS,
    PoseExecutionError,
    SHOULDER_POSE_TORQUE_LIMIT_RAW,
    build_hardware_pose_plan,
    execute_pose_cycle,
)


DEFAULT_CALIBRATION = Path("~/.config/orion/servo_calibration.json")
DEFAULT_POSES = (
    Path(__file__).resolve().parents[3]
    / "motion"
    / "config"
    / "poses.yaml"
)


def _duration(text: str) -> float:
    value = float(text)
    if not math.isfinite(value) or value < MIN_POSE_DURATION_SECONDS:
        raise argparse.ArgumentTypeError(
            f"must be a finite number at least {MIN_POSE_DURATION_SECONDS:.1f}"
        )
    return value


def _nonnegative(text: str) -> float:
    value = float(text)
    if not math.isfinite(value) or value < 0:
        raise argparse.ArgumentTypeError("must be a finite non-negative number")
    return value


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Commission one physical Orion named pose: move from the measured safe start, "
            "hold, return to calibrated zero until Ctrl+C, then park at the captured "
            "torque-free rest pose and disable torque."
        )
    )
    parser.add_argument("pose", help="Pose name from Orion's poses.yaml, initially use 'home'.")
    parser.add_argument("--port", required=True, help="Servo adapter serial port.")
    parser.add_argument("--calibration", type=Path, default=DEFAULT_CALIBRATION)
    parser.add_argument("--poses", type=Path, default=DEFAULT_POSES)
    parser.add_argument("--duration", type=_duration, default=6.0)
    parser.add_argument("--hold", type=_nonnegative, default=2.0)
    parser.add_argument("--return-duration", type=_duration, default=6.0)
    parser.add_argument("--rest-duration", type=_duration, default=6.0)
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Resolve and print raw targets without opening the servo bus.",
    )
    return parser


def _print_plan(plan) -> None:
    print(f"Orion physical named-pose commissioning plan: {plan.pose_name}")
    print(f"Pose source: {plan.pose_path}")
    print(f"Calibration: {plan.calibration_path.expanduser()}")
    print("  joint                         radians   delta_raw  target_raw")
    for target in plan.targets:
        print(
            f"  {target.calibration.joint_name:<29} "
            f"{target.angle_radians:+8.3f} {target.delta_raw:+11d} {target.target_raw:11d}"
        )
    print(
        f"Pose torque limits: shoulder {SHOULDER_POSE_TORQUE_LIMIT_RAW}/1000; "
        "all other joints 200/1000."
    )
    print(f"Pose temperature cutoff: {MAX_POSE_TEMPERATURE_C} C; current cutoff: 1 A.")


class EmergencyTermination(BaseException):
    """Terminal loss or process termination that must not start planned motion."""


def _raise_emergency_termination(_signum, _frame) -> None:
    """Route terminal loss and termination through immediate torque-off cleanup."""

    raise EmergencyTermination


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        plan = build_hardware_pose_plan(
            args.pose,
            pose_path=args.poses,
            calibration_path=args.calibration,
        )
        rest_plan = build_hardware_pose_plan(
            "rest",
            pose_path=args.poses,
            calibration_path=args.calibration,
        )
        _print_plan(plan)
        print("Shutdown target: captured torque-free pose 'rest'.")
    except (OSError, RuntimeError, ValueError) as exc:
        print(f"Cannot prepare hardware pose: {exc}")
        return 1

    if args.dry_run:
        print("Dry run only: no serial port was opened and no servo register was written.")
        return 0

    confirmation = input(
        "\nTurn 6 V OFF. Position Orion anywhere inside its calibrated safe travel over a "
        "clear padded area; remove blocks that obstruct the planned movement. Keep hands clear "
        "and the 6 V cutoff within reach. Turn 6 V ON. If the lamp remains still, type "
        f"RUN {args.pose}: "
    )
    if confirmation.strip() != f"RUN {args.pose}":
        print("Pose cycle cancelled. No serial port was opened.")
        return 2

    bus = None
    previous_signal_handlers = {}
    for signal_name in ("SIGHUP", "SIGTERM"):
        signal_value = getattr(signal, signal_name, None)
        if signal_value is not None:
            previous_signal_handlers[signal_value] = signal.signal(
                signal_value, _raise_emergency_termination
            )
    try:
        assignments = motion_test_plan()
        bus = create_lerobot_bus(args.port, assignments)
        bus.connect(handshake=True)
        read_motion_preflight(bus, assignments)
        print("Preflight passed. Moving from the measured position to the named pose.")
        result = execute_pose_cycle(
            bus,
            plan,
            rest_plan,
            pose_duration=args.duration,
            hold_seconds=args.hold,
            return_duration=args.return_duration,
            rest_duration=args.rest_duration,
            on_zero_hold=lambda: print(
                "Reached calibrated zero and holding with health monitoring. Press Ctrl+C once "
                "to perform the planned move to rest. Keep the path clear; another Ctrl+C while "
                "parking is an emergency torque-off."
            ),
            on_rest_reached=lambda: print(
                "Reached the captured torque-free rest pose. Disabling torque now."
            ),
        )
    except EmergencyTermination:
        print("\nTerminal closed or process terminated. Disabling all torque immediately.")
        return 143
    except KeyboardInterrupt:
        print("\nPose cycle interrupted. Disabling all torque immediately.")
        return 130
    except (ConnectionError, OSError, RuntimeError, ValueError) as exc:
        print(f"\nPose cycle stopped: {exc}")
        print("Disabling all torque. Turn 6 V OFF.")
        return 1
    finally:
        if bus is not None and getattr(bus, "is_connected", False):
            try:
                bus.disable_torque(num_retry=2)
            finally:
                bus.disconnect(disable_torque=True)
        for signal_value, previous_handler in previous_signal_handlers.items():
            signal.signal(signal_value, previous_handler)

    print(
        f"\nPose '{result.pose_name}' cycle complete; peak current "
        f"{result.peak_current_ma:.0f} mA, maximum temperature "
        f"{result.maximum_temperature_c} C."
    )
    print(
        "Parked at the validated mechanical rest pose; all torque is off. Confirm the lamp "
        "remains still, then turn 6 V OFF."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
