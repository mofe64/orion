"""Read-only identity and telemetry checks for provisioned Orion servos."""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass
from typing import Protocol

from .provisioning import ORION_SERVO_ASSIGNMENTS, ServoAssignment, validate_assignments


@dataclass(frozen=True)
class ServoTelemetry:
    """One read-only snapshot from a provisioned STS3215 servo."""

    assignment: ServoAssignment
    model_number: int
    position_raw: int
    voltage_v: float
    temperature_c: int
    torque_enabled: bool


class VerificationBus(Protocol):
    """Read-only part of LeRobot's motor-bus API used by verification."""

    def ping(self, motor: str, num_retry: int = 0, raise_on_error: bool = False) -> int | None: ...

    def read(
        self,
        data_name: str,
        motor: str,
        *,
        normalize: bool = True,
        num_retry: int = 0,
    ) -> int: ...


def verification_plan(
    assignments: Iterable[ServoAssignment] = ORION_SERVO_ASSIGNMENTS,
    *,
    selected_joint: str | None = None,
) -> tuple[ServoAssignment, ...]:
    """Return the expected ID map in ascending bus-ID order."""

    validated = validate_assignments(assignments)
    if selected_joint is None:
        return tuple(sorted(validated, key=lambda item: item.servo_id))

    matches = tuple(item for item in validated if item.joint_name == selected_joint)
    if not matches:
        choices = ", ".join(item.joint_name for item in validated)
        raise ValueError(f"Unknown joint '{selected_joint}'. Expected one of: {choices}")
    return matches


def read_servo_telemetry(
    bus: VerificationBus,
    assignments: Iterable[ServoAssignment],
) -> tuple[ServoTelemetry, ...]:
    """Ping and read each servo without changing any motor register."""

    snapshots: list[ServoTelemetry] = []
    for assignment in validate_assignments(assignments):
        model_number = bus.ping(assignment.joint_name, num_retry=2, raise_on_error=True)
        if model_number is None:
            raise ConnectionError(
                f"Servo ID {assignment.servo_id} ({assignment.joint_name}) did not answer its ping."
            )

        position_raw = int(
            bus.read("Present_Position", assignment.joint_name, normalize=False, num_retry=2)
        )
        voltage_raw = int(
            bus.read("Present_Voltage", assignment.joint_name, normalize=False, num_retry=2)
        )
        temperature_c = int(
            bus.read("Present_Temperature", assignment.joint_name, normalize=False, num_retry=2)
        )
        torque_raw = int(bus.read("Torque_Enable", assignment.joint_name, normalize=False, num_retry=2))

        snapshots.append(
            ServoTelemetry(
                assignment=assignment,
                model_number=int(model_number),
                position_raw=position_raw,
                voltage_v=voltage_raw / 10.0,
                temperature_c=temperature_c,
                torque_enabled=bool(torque_raw),
            )
        )

    return tuple(snapshots)
