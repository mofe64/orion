"""Read-only register audit for Orion's provisioned STS3215 servos."""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass
from typing import Protocol

from .provisioning import ORION_SERVO_ASSIGNMENTS, ServoAssignment, validate_assignments


REGISTER_GROUPS: tuple[tuple[str, tuple[str, ...]], ...] = (
    (
        "identity",
        (
            "Firmware_Major_Version",
            "Firmware_Minor_Version",
            "ID",
            "Baud_Rate",
        ),
    ),
    (
        "persistent configuration",
        (
            "Return_Delay_Time",
            "Min_Position_Limit",
            "Max_Position_Limit",
            "Max_Temperature_Limit",
            "Max_Voltage_Limit",
            "Min_Voltage_Limit",
            "Max_Torque_Limit",
            "Phase",
            "P_Coefficient",
            "I_Coefficient",
            "D_Coefficient",
            "Protection_Current",
            "Homing_Offset",
            "Operating_Mode",
            "Protective_Torque",
            "Protection_Time",
            "Overload_Torque",
            "Over_Current_Protection_Time",
            "Maximum_Velocity_Limit",
            "Maximum_Acceleration",
        ),
    ),
    (
        "runtime configuration",
        (
            "Torque_Enable",
            "Acceleration",
            "Goal_Position",
            "Goal_Velocity",
            "Torque_Limit",
            "Lock",
        ),
    ),
    (
        "live telemetry",
        (
            "Present_Position",
            "Present_Velocity",
            "Present_Load",
            "Present_Voltage",
            "Present_Temperature",
            "Status",
            "Moving",
            "Present_Current",
        ),
    ),
)

AUDIT_REGISTERS = tuple(
    register for _, registers in REGISTER_GROUPS for register in registers
)


@dataclass(frozen=True)
class ServoRegisterSnapshot:
    """Raw register values captured from one STS3215 without writing to it."""

    assignment: ServoAssignment
    model_number: int
    registers: dict[str, int]

    def raw(self, register: str) -> int:
        return self.registers[register]

    @property
    def firmware_version(self) -> str:
        return (
            f"{self.raw('Firmware_Major_Version')}."
            f"{self.raw('Firmware_Minor_Version')}"
        )

    @property
    def position_raw(self) -> int:
        return self.raw("Present_Position")

    @property
    def voltage_v(self) -> float:
        return self.raw("Present_Voltage") / 10.0

    @property
    def temperature_c(self) -> int:
        return self.raw("Present_Temperature")

    @property
    def current_ma(self) -> float:
        return self.raw("Present_Current") * 6.5

    @property
    def torque_enabled(self) -> bool:
        return bool(self.raw("Torque_Enable"))


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


def read_servo_registers(
    bus: VerificationBus,
    assignments: Iterable[ServoAssignment],
) -> tuple[ServoRegisterSnapshot, ...]:
    """Ping and audit each servo using read operations only."""

    snapshots: list[ServoRegisterSnapshot] = []
    for assignment in validate_assignments(assignments):
        model_number = bus.ping(assignment.joint_name, num_retry=2, raise_on_error=True)
        if model_number is None:
            raise ConnectionError(
                f"Servo ID {assignment.servo_id} ({assignment.joint_name}) did not answer its ping."
            )

        registers = {
            register: int(
                bus.read(
                    register,
                    assignment.joint_name,
                    normalize=False,
                    num_retry=2,
                )
            )
            for register in AUDIT_REGISTERS
        }

        snapshots.append(
            ServoRegisterSnapshot(
                assignment=assignment,
                model_number=int(model_number),
                registers=registers,
            )
        )

    return tuple(snapshots)
