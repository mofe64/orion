"""Calibration-aware named-pose commissioning for physical Orion hardware.

This module deliberately contains no ROS or LeRobot imports. It maps Orion's
existing radian pose definitions into raw STS3215 targets, enforces the measured
hardware calibration, and provides a guarded pose/return cycle that can be unit
tested with a fake bus.
"""

from __future__ import annotations

import json
import math
import time
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol

import yaml

from .calibration import ENCODER_RESOLUTION, circular_delta
from .motion_test import (
    ACCELERATION_RAW,
    GOAL_VELOCITY_RAW,
    MAX_TEST_CURRENT_RAW,
    TORQUE_LIMIT_RAW,
)
from .provisioning import ORION_SERVO_ASSIGNMENTS


STEPS_PER_RADIAN = ENCODER_RESOLUTION / (2.0 * math.pi)
POSITION_TOLERANCE_RAW = 20  # About 1.76 degrees at phase completion.
CONTROL_INTERVAL_SECONDS = 0.10
MIN_POSE_DURATION_SECONDS = 4.0
SHOULDER_POSE_TORQUE_LIMIT_RAW = 400
MAX_POSE_TEMPERATURE_C = 50


class PoseExecutionError(RuntimeError):
    """Raised when a pose cannot be planned or executed safely."""


class PoseBus(Protocol):
    """LeRobot motor-bus operations used by the commissioning pose cycle."""

    def read(
        self,
        data_name: str,
        motor: str,
        *,
        normalize: bool = True,
        num_retry: int = 0,
    ) -> int: ...

    def write(
        self,
        data_name: str,
        motor: str,
        value: int,
        *,
        normalize: bool = True,
        num_retry: int = 0,
    ) -> None: ...

    def sync_read(
        self,
        data_name: str,
        motors=None,
        *,
        normalize: bool = True,
        num_retry: int = 0,
    ) -> dict[str, int]: ...

    def sync_write(
        self,
        data_name: str,
        values: Mapping[str, int],
        *,
        normalize: bool = True,
        num_retry: int = 0,
    ) -> None: ...

    def enable_torque(self, motors=None, num_retry: int = 0) -> None: ...

    def disable_torque(self, motors=None, num_retry: int = 0) -> None: ...


@dataclass(frozen=True)
class HardwareJointCalibration:
    joint_name: str
    servo_id: int
    neutral_raw: int
    encoder_direction: int
    safe_min_delta_raw: int
    safe_max_delta_raw: int


@dataclass(frozen=True)
class PoseJointTarget:
    calibration: HardwareJointCalibration
    angle_radians: float
    delta_raw: int
    target_raw: int


@dataclass(frozen=True)
class HardwarePosePlan:
    pose_name: str
    pose_path: Path
    calibration_path: Path
    targets: tuple[PoseJointTarget, ...]

    @property
    def target_positions(self) -> dict[str, int]:
        return {
            target.calibration.joint_name: target.target_raw
            for target in self.targets
        }

    @property
    def neutral_positions(self) -> dict[str, int]:
        return {
            target.calibration.joint_name: target.calibration.neutral_raw
            for target in self.targets
        }


@dataclass(frozen=True)
class PoseCycleResult:
    pose_name: str
    peak_current_ma: float
    maximum_temperature_c: int


def _require_int(value: Any, path: str) -> int:
    if type(value) is not int:
        raise PoseExecutionError(f"{path} must be an integer.")
    return value


def load_hardware_calibration(path: Path) -> dict[str, HardwareJointCalibration]:
    """Load and validate the software calibration needed for pose conversion."""

    path = path.expanduser()
    try:
        root = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise PoseExecutionError(f"Could not read calibration '{path}': {exc}") from exc
    if not isinstance(root, dict) or root.get("schema_version") != 1:
        raise PoseExecutionError("Calibration must use schema_version 1.")
    if root.get("robot") != "orion" or root.get("servo_model") != "sts3215":
        raise PoseExecutionError("Calibration is not for Orion STS3215 hardware.")
    if root.get("writes_servo_eeprom") is not False:
        raise PoseExecutionError("Calibration provenance is missing software-only EEPROM state.")

    raw_joints = root.get("joints")
    if not isinstance(raw_joints, dict):
        raise PoseExecutionError("Calibration joints must be a mapping.")
    expected_names = {item.joint_name for item in ORION_SERVO_ASSIGNMENTS}
    if set(raw_joints) != expected_names:
        raise PoseExecutionError("Calibration must contain Orion's five canonical joints.")

    result: dict[str, HardwareJointCalibration] = {}
    for assignment in ORION_SERVO_ASSIGNMENTS:
        raw = raw_joints[assignment.joint_name]
        if not isinstance(raw, dict):
            raise PoseExecutionError(f"Calibration {assignment.joint_name} must be a mapping.")
        servo_id = _require_int(raw.get("servo_id"), f"{assignment.joint_name}.servo_id")
        neutral = _require_int(raw.get("neutral_raw"), f"{assignment.joint_name}.neutral_raw")
        direction = _require_int(
            raw.get("encoder_direction"), f"{assignment.joint_name}.encoder_direction"
        )
        safe_min = _require_int(
            raw.get("safe_min_delta_raw"), f"{assignment.joint_name}.safe_min_delta_raw"
        )
        safe_max = _require_int(
            raw.get("safe_max_delta_raw"), f"{assignment.joint_name}.safe_max_delta_raw"
        )
        if servo_id != assignment.servo_id:
            raise PoseExecutionError(
                f"{assignment.joint_name} calibration ID {servo_id} does not match ID "
                f"{assignment.servo_id}."
            )
        if not 0 <= neutral < ENCODER_RESOLUTION:
            raise PoseExecutionError(f"{assignment.joint_name} neutral is outside 0..4095.")
        if direction not in (-1, 1):
            raise PoseExecutionError(f"{assignment.joint_name} direction must be -1 or +1.")
        if not safe_min < 0 < safe_max:
            raise PoseExecutionError(
                f"{assignment.joint_name} safe range must contain calibrated zero."
            )
        result[assignment.joint_name] = HardwareJointCalibration(
            joint_name=assignment.joint_name,
            servo_id=servo_id,
            neutral_raw=neutral,
            encoder_direction=direction,
            safe_min_delta_raw=safe_min,
            safe_max_delta_raw=safe_max,
        )
    return result


def load_named_pose(path: Path, pose_name: str) -> dict[str, float]:
    """Load one complete radian pose from Orion's existing pose library."""

    try:
        root = yaml.safe_load(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, yaml.YAMLError) as exc:
        raise PoseExecutionError(f"Could not read pose library '{path}': {exc}") from exc
    if not isinstance(root, dict) or root.get("format_version") != 1:
        raise PoseExecutionError("Pose library must use format_version 1.")
    if root.get("units") != "radians":
        raise PoseExecutionError("Pose library units must be radians.")
    poses = root.get("poses")
    if not isinstance(poses, dict) or pose_name not in poses:
        choices = ", ".join(sorted(poses)) if isinstance(poses, dict) else "none"
        raise PoseExecutionError(f"Unknown pose '{pose_name}'. Available poses: {choices}.")
    pose = poses[pose_name]
    positions = pose.get("positions") if isinstance(pose, dict) else None
    if not isinstance(positions, dict):
        raise PoseExecutionError(f"Pose '{pose_name}' positions must be a mapping.")
    expected_names = {item.joint_name for item in ORION_SERVO_ASSIGNMENTS}
    if set(positions) != expected_names:
        raise PoseExecutionError(f"Pose '{pose_name}' must contain Orion's five joints.")

    result: dict[str, float] = {}
    for joint_name, value in positions.items():
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise PoseExecutionError(f"Pose '{pose_name}' {joint_name} must be numeric.")
        angle = float(value)
        if not math.isfinite(angle):
            raise PoseExecutionError(f"Pose '{pose_name}' {joint_name} must be finite.")
        result[joint_name] = angle
    return result


def build_hardware_pose_plan(
    pose_name: str,
    *,
    pose_path: Path,
    calibration_path: Path,
) -> HardwarePosePlan:
    """Convert a named Orion pose into bounded raw STS3215 targets."""

    calibration = load_hardware_calibration(calibration_path)
    positions = load_named_pose(pose_path, pose_name)
    targets: list[PoseJointTarget] = []
    for assignment in ORION_SERVO_ASSIGNMENTS:
        joint = calibration[assignment.joint_name]
        angle = positions[assignment.joint_name]
        delta = round(angle * STEPS_PER_RADIAN) * joint.encoder_direction
        if not joint.safe_min_delta_raw <= delta <= joint.safe_max_delta_raw:
            raise PoseExecutionError(
                f"Pose '{pose_name}' requests {assignment.joint_name} delta {delta}, outside "
                f"calibrated [{joint.safe_min_delta_raw}, {joint.safe_max_delta_raw}]."
            )
        target = joint.neutral_raw + delta
        if not 0 <= target < ENCODER_RESOLUTION:
            raise PoseExecutionError(
                f"Pose '{pose_name}' would cross the raw encoder boundary for "
                f"{assignment.joint_name}; commissioning execution rejects wraparound targets."
            )
        targets.append(PoseJointTarget(joint, angle, delta, target))
    return HardwarePosePlan(
        pose_name=pose_name,
        pose_path=pose_path,
        calibration_path=calibration_path,
        targets=tuple(targets),
    )


def _quintic_fraction(value: float) -> float:
    return 10.0 * value**3 - 15.0 * value**4 + 6.0 * value**5


def _read_health(bus: PoseBus) -> tuple[float, int]:
    currents = bus.sync_read("Present_Current", normalize=False, num_retry=2)
    temperatures = bus.sync_read("Present_Temperature", normalize=False, num_retry=2)
    statuses = bus.sync_read("Status", normalize=False, num_retry=2)
    peak_current = max(abs(int(value)) for value in currents.values())
    maximum_temperature = max(int(value) for value in temperatures.values())
    if peak_current > MAX_TEST_CURRENT_RAW:
        raise PoseExecutionError("A servo exceeded the 1.0 A commissioning current limit.")
    hot_servos = {
        name: int(value)
        for name, value in temperatures.items()
        if int(value) > MAX_POSE_TEMPERATURE_C
    }
    if hot_servos:
        details = ", ".join(f"{name}={value} C" for name, value in hot_servos.items())
        raise PoseExecutionError(
            f"Servo temperature exceeded the {MAX_POSE_TEMPERATURE_C} C pose limit: "
            f"{details}."
        )
    faults = {name: int(value) for name, value in statuses.items() if int(value) != 0}
    if faults:
        details = ", ".join(f"{name}=0x{value:02x}" for name, value in faults.items())
        raise PoseExecutionError(f"Servo fault during pose cycle: {details}.")
    return peak_current * 6.5, maximum_temperature


def _move_interpolated(
    bus: PoseBus,
    start: Mapping[str, int],
    target: Mapping[str, int],
    duration: float,
    *,
    sleep: Callable[[float], None],
) -> tuple[float, int]:
    steps = max(1, math.ceil(duration / CONTROL_INTERVAL_SECONDS))
    peak_current_ma = 0.0
    maximum_temperature = 0
    for step in range(1, steps + 1):
        fraction = _quintic_fraction(step / steps)
        goals = {
            name: round(start[name] + (target[name] - start[name]) * fraction)
            for name in target
        }
        bus.sync_write("Goal_Position", goals, normalize=False, num_retry=2)
        sleep(duration / steps)
        current_ma, temperature = _read_health(bus)
        peak_current_ma = max(peak_current_ma, current_ma)
        maximum_temperature = max(maximum_temperature, temperature)

    final_positions = bus.sync_read("Present_Position", normalize=False, num_retry=2)
    errors = {
        name: abs(int(final_positions[name]) - expected)
        for name, expected in target.items()
        if abs(int(final_positions[name]) - expected) > POSITION_TOLERANCE_RAW
    }
    if errors:
        details = ", ".join(
            f"{name}={error} steps (actual {int(final_positions[name])}, target {target[name]})"
            for name, error in errors.items()
        )
        raise PoseExecutionError(
            f"Pose tracking error exceeded tolerance: {details}; phase peak current "
            f"{peak_current_ma:.0f} mA, maximum temperature {maximum_temperature} C."
        )
    return peak_current_ma, maximum_temperature


def execute_pose_cycle(
    bus: PoseBus,
    plan: HardwarePosePlan,
    rest_plan: HardwarePosePlan,
    *,
    pose_duration: float,
    hold_seconds: float,
    return_duration: float,
    rest_duration: float,
    sleep: Callable[[float], None] = time.sleep,
    should_leave_zero_hold: Callable[[], bool] = lambda: False,
    on_zero_hold: Callable[[], None] = lambda: None,
    on_rest_reached: Callable[[], None] = lambda: None,
) -> PoseCycleResult:
    """Visit a pose, hold zero until interrupted, then park at rest and disable."""

    durations = (pose_duration, return_duration, rest_duration)
    if any(
        not math.isfinite(value) or value < MIN_POSE_DURATION_SECONDS
        for value in durations
    ):
        raise PoseExecutionError(
            "Pose, zero-return, and rest-parking durations must each be at least "
            f"{MIN_POSE_DURATION_SECONDS:.1f} s."
        )
    if not math.isfinite(hold_seconds) or hold_seconds < 0:
        raise PoseExecutionError("Hold duration must be finite and non-negative.")
    if rest_plan.pose_name != "rest":
        raise PoseExecutionError("Shutdown plan must be the captured 'rest' pose.")
    if tuple(target.calibration for target in plan.targets) != tuple(
        target.calibration for target in rest_plan.targets
    ):
        raise PoseExecutionError("Pose and rest plans must use the same hardware calibration.")

    current = {
        name: int(value)
        for name, value in bus.sync_read(
            "Present_Position", normalize=False, num_retry=2
        ).items()
    }
    for target in plan.targets:
        name = target.calibration.joint_name
        delta = circular_delta(current[name], target.calibration.neutral_raw)
        if not (
            target.calibration.safe_min_delta_raw
            <= delta
            <= target.calibration.safe_max_delta_raw
        ):
            raise PoseExecutionError(
                f"{name} starts at delta {delta:+d}, outside its calibrated safe range "
                f"[{target.calibration.safe_min_delta_raw}, "
                f"{target.calibration.safe_max_delta_raw}]."
            )
        unwrapped_position = target.calibration.neutral_raw + delta
        if unwrapped_position != current[name]:
            raise PoseExecutionError(
                f"{name} starts across the raw 0/4095 encoder boundary. Manually reposition "
                "it closer to calibrated zero before commissioning pose motion."
            )

    for target in plan.targets:
        motor = target.calibration.joint_name
        torque_limit = (
            SHOULDER_POSE_TORQUE_LIMIT_RAW
            if motor == "shoulder_pitch_joint"
            else TORQUE_LIMIT_RAW
        )
        bus.write("Acceleration", motor, ACCELERATION_RAW, normalize=False, num_retry=2)
        bus.write("Goal_Velocity", motor, GOAL_VELOCITY_RAW, normalize=False, num_retry=2)
        bus.write("Torque_Limit", motor, torque_limit, normalize=False, num_retry=2)
    bus.sync_write("Goal_Position", current, normalize=False, num_retry=2)

    peak_current_ma = 0.0
    maximum_temperature = 0
    torque_enabled = False
    try:
        bus.enable_torque(num_retry=2)
        torque_enabled = True
        sleep(0.20)
        current_ma, temperature = _read_health(bus)
        peak_current_ma = max(peak_current_ma, current_ma)
        maximum_temperature = max(maximum_temperature, temperature)

        pose_peak, pose_temp = _move_interpolated(
            bus,
            current,
            plan.target_positions,
            pose_duration,
            sleep=sleep,
        )
        peak_current_ma = max(peak_current_ma, pose_peak)
        maximum_temperature = max(maximum_temperature, pose_temp)

        if hold_seconds:
            hold_steps = max(1, math.ceil(hold_seconds / CONTROL_INTERVAL_SECONDS))
            for _ in range(hold_steps):
                sleep(hold_seconds / hold_steps)
                current_ma, temperature = _read_health(bus)
                peak_current_ma = max(peak_current_ma, current_ma)
                maximum_temperature = max(maximum_temperature, temperature)

        return_peak, return_temp = _move_interpolated(
            bus,
            plan.target_positions,
            plan.neutral_positions,
            return_duration,
            sleep=sleep,
        )
        peak_current_ma = max(peak_current_ma, return_peak)
        maximum_temperature = max(maximum_temperature, return_temp)
        on_zero_hold()
        try:
            while not should_leave_zero_hold():
                sleep(CONTROL_INTERVAL_SECONDS)
                current_ma, temperature = _read_health(bus)
                peak_current_ma = max(peak_current_ma, current_ma)
                maximum_temperature = max(maximum_temperature, temperature)
        except KeyboardInterrupt:
            # Ctrl+C has a planned meaning only after zero has been reached:
            # leave the zero hold and perform a controlled park at rest.
            pass

        rest_peak, rest_temp = _move_interpolated(
            bus,
            plan.neutral_positions,
            rest_plan.target_positions,
            rest_duration,
            sleep=sleep,
        )
        peak_current_ma = max(peak_current_ma, rest_peak)
        maximum_temperature = max(maximum_temperature, rest_temp)
        on_rest_reached()
    finally:
        if torque_enabled:
            bus.disable_torque(num_retry=2)

    return PoseCycleResult(
        plan.pose_name,
        peak_current_ma,
        maximum_temperature,
    )
