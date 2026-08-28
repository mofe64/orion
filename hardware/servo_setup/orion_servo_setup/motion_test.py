"""Conservative first-motion checks for an assembled Orion lamp.

The first powered movement is deliberately different from runtime control:
only one joint is torqued at a time, the target starts at the encoder's current
position, and every nudge is small, slow, and torque-limited.  This module has
no LeRobot imports so its safety sequencing can be unit tested with a fake bus.
"""

from __future__ import annotations

import time
from collections.abc import Callable, Iterable
from dataclasses import dataclass
from typing import Protocol

from .provisioning import ORION_SERVO_ASSIGNMENTS, ServoAssignment, validate_assignments


MODEL_NUMBER_STS3215 = 777
MIN_SUPPLY_RAW = 55  # 5.5 V; Orion's nominal servo rail is 6 V.
MAX_SUPPLY_RAW = 66  # 6.6 V; reject the wrong supply before enabling torque.
MAX_START_TEMPERATURE_C = 40
MAX_TEST_TEMPERATURE_C = 45
MAX_TEST_CURRENT_RAW = 154  # 154 * 6.5 mA ~= 1.0 A.

NUDGE_STEPS = 10  # 4096 steps/rev: about 0.88 degrees.
POSITION_TOLERANCE_STEPS = 3
MIN_DIRECTIONAL_RESPONSE_STEPS = 3
GOAL_VELOCITY_RAW = 50  # About 4.4 degrees/second.
ACCELERATION_RAW = 5
TORQUE_LIMIT_RAW = 200  # 20% of the STS3215's 0..1000 RAM torque limit.
POLL_INTERVAL_SECONDS = 0.05
POLL_ATTEMPTS = 30


class MotionTestError(RuntimeError):
    """Raised when a first-motion safety invariant is not satisfied."""


class MotionTestBus(Protocol):
    """LeRobot motor-bus operations used by the first-motion workflow."""

    def ping(self, motor: str, num_retry: int = 0, raise_on_error: bool = False) -> int | None: ...

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

    def enable_torque(self, motors: int | str | list[str] | None = None, num_retry: int = 0) -> None: ...

    def disable_torque(self, motors: int | str | list[str] | None = None, num_retry: int = 0) -> None: ...


@dataclass(frozen=True)
class MotionPreflight:
    """Read-only state required before any joint may be enabled."""

    assignment: ServoAssignment
    position_raw: int
    voltage_v: float
    temperature_c: int


@dataclass(frozen=True)
class MotionResult:
    """Observed result of one automatically torque-disabled nudge."""

    assignment: ServoAssignment
    start_position_raw: int
    target_position_raw: int
    final_position_raw: int
    peak_current_ma: float
    final_temperature_c: int
    reached_target: bool


def motion_test_plan(
    assignments: Iterable[ServoAssignment] = ORION_SERVO_ASSIGNMENTS,
) -> tuple[ServoAssignment, ...]:
    """Return all joints in ascending bus-ID order."""

    return tuple(sorted(validate_assignments(assignments), key=lambda item: item.servo_id))


def read_motion_preflight(
    bus: MotionTestBus,
    assignments: Iterable[ServoAssignment],
) -> tuple[MotionPreflight, ...]:
    """Reject unsafe bus state before the first torque-enable write."""

    snapshots: list[MotionPreflight] = []
    for assignment in validate_assignments(assignments):
        motor = assignment.joint_name
        model_number = bus.ping(motor, num_retry=2, raise_on_error=True)
        if model_number != MODEL_NUMBER_STS3215:
            raise MotionTestError(
                f"ID {assignment.servo_id} reported model {model_number}; expected STS3215 model 777."
            )

        operating_mode = int(bus.read("Operating_Mode", motor, normalize=False, num_retry=2))
        if operating_mode != 0:
            raise MotionTestError(
                f"ID {assignment.servo_id} is in operating mode {operating_mode}, not position mode 0."
            )

        torque_enabled = int(bus.read("Torque_Enable", motor, normalize=False, num_retry=2))
        if torque_enabled != 0:
            raise MotionTestError(
                f"ID {assignment.servo_id} already has torque enabled. Power off and investigate before testing."
            )

        position_raw = int(bus.read("Present_Position", motor, normalize=False, num_retry=2))
        voltage_raw = int(bus.read("Present_Voltage", motor, normalize=False, num_retry=2))
        temperature_c = int(
            bus.read("Present_Temperature", motor, normalize=False, num_retry=2)
        )
        status = int(bus.read("Status", motor, normalize=False, num_retry=2))

        if not 0 <= position_raw <= 4095:
            raise MotionTestError(
                f"ID {assignment.servo_id} returned invalid raw position {position_raw}."
            )
        if not MIN_SUPPLY_RAW <= voltage_raw <= MAX_SUPPLY_RAW:
            raise MotionTestError(
                f"ID {assignment.servo_id} reports {voltage_raw / 10:.1f} V; "
                "the Orion first-motion window is 5.5-6.6 V."
            )
        if temperature_c > MAX_START_TEMPERATURE_C:
            raise MotionTestError(
                f"ID {assignment.servo_id} is already {temperature_c} C; allow it to cool below "
                f"{MAX_START_TEMPERATURE_C} C."
            )
        if status != 0:
            raise MotionTestError(
                f"ID {assignment.servo_id} reports servo status 0x{status:02x}; clear the fault first."
            )

        snapshots.append(
            MotionPreflight(
                assignment=assignment,
                position_raw=position_raw,
                voltage_v=voltage_raw / 10.0,
                temperature_c=temperature_c,
            )
        )

    return tuple(snapshots)


def nudge_joint(
    bus: MotionTestBus,
    assignment: ServoAssignment,
    *,
    direction: int,
    sleep: Callable[[float], None] = time.sleep,
) -> MotionResult:
    """Hold at the present encoder value, nudge once, and always disable torque."""

    if direction not in (-1, 1):
        raise ValueError("Nudge direction must be -1 or +1.")

    motor = assignment.joint_name
    start_position = int(bus.read("Present_Position", motor, normalize=False, num_retry=2))
    target_position = start_position + direction * NUDGE_STEPS
    if not 0 <= target_position <= 4095:
        raise MotionTestError(
            f"ID {assignment.servo_id} cannot nudge {direction:+d}: raw target "
            f"{target_position} is outside 0..4095. Choose the opposite direction."
        )

    # All four writes below affect RAM only.  The present position becomes the
    # goal before torque is enabled, preventing a jump to a stale target.
    bus.write("Acceleration", motor, ACCELERATION_RAW, normalize=False, num_retry=2)
    bus.write("Goal_Velocity", motor, GOAL_VELOCITY_RAW, normalize=False, num_retry=2)
    bus.write("Torque_Limit", motor, TORQUE_LIMIT_RAW, normalize=False, num_retry=2)
    bus.write("Goal_Position", motor, start_position, normalize=False, num_retry=2)

    torque_enabled = False
    peak_current_raw = 0
    final_position = start_position
    final_temperature = 0
    try:
        bus.enable_torque(motor, num_retry=2)
        torque_enabled = True
        sleep(0.15)

        held_position = int(bus.read("Present_Position", motor, normalize=False, num_retry=2))
        if abs(held_position - start_position) > POSITION_TOLERANCE_STEPS:
            raise MotionTestError(
                f"ID {assignment.servo_id} shifted from {start_position} to {held_position} while "
                "acquiring its current-position hold."
            )

        bus.write("Goal_Position", motor, target_position, normalize=False, num_retry=2)
        reached_target = False
        for _ in range(POLL_ATTEMPTS):
            sleep(POLL_INTERVAL_SECONDS)
            final_position = int(
                bus.read("Present_Position", motor, normalize=False, num_retry=2)
            )
            current_raw = int(bus.read("Present_Current", motor, normalize=False, num_retry=2))
            final_temperature = int(
                bus.read("Present_Temperature", motor, normalize=False, num_retry=2)
            )
            status = int(bus.read("Status", motor, normalize=False, num_retry=2))
            peak_current_raw = max(peak_current_raw, current_raw)

            if current_raw > MAX_TEST_CURRENT_RAW:
                raise MotionTestError(
                    f"ID {assignment.servo_id} exceeded the 1.0 A first-motion current limit."
                )
            if final_temperature > MAX_TEST_TEMPERATURE_C:
                raise MotionTestError(
                    f"ID {assignment.servo_id} reached {final_temperature} C during its nudge."
                )
            if status != 0:
                raise MotionTestError(
                    f"ID {assignment.servo_id} reported status 0x{status:02x} during its nudge."
                )
            if abs(final_position - target_position) <= POSITION_TOLERANCE_STEPS:
                reached_target = True
                break

        directional_response = direction * (final_position - start_position)
        if not reached_target and directional_response < MIN_DIRECTIONAL_RESPONSE_STEPS:
            raise MotionTestError(
                f"ID {assignment.servo_id} did not show a clear response toward raw target "
                f"{target_position}; it moved from {start_position} to {final_position}."
            )
    finally:
        if torque_enabled:
            bus.disable_torque(motor, num_retry=2)

    return MotionResult(
        assignment=assignment,
        start_position_raw=start_position,
        target_position_raw=target_position,
        final_position_raw=final_position,
        peak_current_ma=peak_current_raw * 6.5,
        final_temperature_c=final_temperature,
        reached_target=reached_target,
    )
