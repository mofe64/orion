"""Read-only servo preflight for torque-off calibration and rest commissioning."""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass
from typing import Protocol

from .provisioning import ORION_SERVO_ASSIGNMENTS, ServoAssignment, validate_assignments


MODEL_NUMBER_STS3215 = 777
MIN_SUPPLY_RAW = 55  # 5.5 V; Orion's nominal servo rail is 6 V.
MAX_SUPPLY_RAW = 66  # 6.6 V; reject the wrong supply before enabling torque.
MAX_START_TEMPERATURE_C = 40


class PreflightError(RuntimeError):
    """Raised when a commissioning preflight invariant is not satisfied."""


class PreflightBus(Protocol):
    """Read-only bus operations used by commissioning checks."""

    def ping(self, motor: str, num_retry: int = 0, raise_on_error: bool = False) -> int | None: ...

    def read(
        self,
        data_name: str,
        motor: str,
        *,
        normalize: bool = True,
        num_retry: int = 0,
    ) -> int: ...


@dataclass(frozen=True)
class ServoPreflight:
    """Read-only state required before any joint may be enabled."""

    assignment: ServoAssignment
    position_raw: int
    voltage_v: float
    temperature_c: int


def commissioning_plan(
    assignments: Iterable[ServoAssignment] = ORION_SERVO_ASSIGNMENTS,
) -> tuple[ServoAssignment, ...]:
    """Return all joints in ascending bus-ID order."""

    return tuple(sorted(validate_assignments(assignments), key=lambda item: item.servo_id))


def read_preflight(
    bus: PreflightBus,
    assignments: Iterable[ServoAssignment],
) -> tuple[ServoPreflight, ...]:
    """Reject unsafe bus state before the first torque-enable write."""

    snapshots: list[ServoPreflight] = []
    for assignment in validate_assignments(assignments):
        motor = assignment.joint_name
        model_number = bus.ping(motor, num_retry=2, raise_on_error=True)
        if model_number != MODEL_NUMBER_STS3215:
            raise PreflightError(
                f"ID {assignment.servo_id} reported model {model_number}; expected STS3215 model 777."
            )

        operating_mode = int(bus.read("Operating_Mode", motor, normalize=False, num_retry=2))
        if operating_mode != 0:
            raise PreflightError(
                f"ID {assignment.servo_id} is in operating mode {operating_mode}, not position mode 0."
            )

        torque_enabled = int(bus.read("Torque_Enable", motor, normalize=False, num_retry=2))
        if torque_enabled != 0:
            raise PreflightError(
                f"ID {assignment.servo_id} already has torque enabled. Power off and investigate before testing."
            )

        position_raw = int(bus.read("Present_Position", motor, normalize=False, num_retry=2))
        voltage_raw = int(bus.read("Present_Voltage", motor, normalize=False, num_retry=2))
        temperature_c = int(
            bus.read("Present_Temperature", motor, normalize=False, num_retry=2)
        )
        status = int(bus.read("Status", motor, normalize=False, num_retry=2))

        if not 0 <= position_raw <= 4095:
            raise PreflightError(
                f"ID {assignment.servo_id} returned invalid raw position {position_raw}."
            )
        if not MIN_SUPPLY_RAW <= voltage_raw <= MAX_SUPPLY_RAW:
            raise PreflightError(
                f"ID {assignment.servo_id} reports {voltage_raw / 10:.1f} V; "
                "the Orion first-motion window is 5.5-6.6 V."
            )
        if temperature_c > MAX_START_TEMPERATURE_C:
            raise PreflightError(
                f"ID {assignment.servo_id} is already {temperature_c} C; allow it to cool below "
                f"{MAX_START_TEMPERATURE_C} C."
            )
        if status != 0:
            raise PreflightError(
                f"ID {assignment.servo_id} reports servo status 0x{status:02x}; clear the fault first."
            )

        snapshots.append(
            ServoPreflight(
                assignment=assignment,
                position_raw=position_raw,
                voltage_v=voltage_raw / 10.0,
                temperature_c=temperature_c,
            )
        )

    return tuple(snapshots)
