"""Safe, testable orchestration for assigning Orion's STS3215 servo IDs.

This module deliberately contains no LeRobot imports.  ID planning and user
confirmation can therefore be tested without a serial adapter or powered
servo.  The hardware adapter is created at the CLI boundary.
"""

from __future__ import annotations

from collections.abc import Callable, Iterable
from dataclasses import dataclass
from typing import Protocol


@dataclass(frozen=True)
class ServoAssignment:
    """The persistent bus ID assigned to one semantic Orion joint."""

    joint_name: str
    servo_id: int
    joint_ref_name: str


# These IDs preserve the reference LeLamp bus layout while using Orion's
# canonical ROS joint names.  Runtime hardware code must reuse this map rather
# than inventing a second set of IDs.
ORION_SERVO_ASSIGNMENTS: tuple[ServoAssignment, ...] = (
    ServoAssignment("base_yaw_joint", 1, "base_yaw"),
    ServoAssignment("shoulder_pitch_joint", 2, "base_pitch"),
    ServoAssignment("elbow_pitch_joint", 3, "elbow_pitch"),
    ServoAssignment("head_roll_joint", 4, "wrist_roll"),
    ServoAssignment("head_pitch_joint", 5, "wrist_pitch"),
)


class ProvisioningBus(Protocol):
    """Small part of LeRobot's motor-bus API used by provisioning."""

    def setup_motor(self, motor: str) -> None: ...


class ProvisioningCancelled(RuntimeError):
    """Raised when the operator does not explicitly approve an EEPROM write."""


def validate_assignments(assignments: Iterable[ServoAssignment]) -> tuple[ServoAssignment, ...]:
    """Return an immutable, validated assignment collection."""

    result = tuple(assignments)
    if not result:
        raise ValueError("At least one servo assignment is required.")

    joint_names = [assignment.joint_name for assignment in result]
    servo_ids = [assignment.servo_id for assignment in result]
    if len(joint_names) != len(set(joint_names)):
        raise ValueError("Servo assignment joint names must be unique.")
    if len(servo_ids) != len(set(servo_ids)):
        raise ValueError("Servo assignment IDs must be unique.")
    if any(not 1 <= servo_id <= 252 for servo_id in servo_ids):
        raise ValueError("STS3215 IDs must be between 1 and 252.")
    return result


def provisioning_plan(
    assignments: Iterable[ServoAssignment] = ORION_SERVO_ASSIGNMENTS,
    *,
    selected_joint: str | None = None,
) -> tuple[ServoAssignment, ...]:
    """Build the write order, programming the factory-default ID 1 last.

    A new STS3215 commonly arrives as ID 1.  Programming IDs 5 through 2
    first keeps the target ID 1 until the final step and makes recovery easier
    if setup is interrupted.  Only one servo may be connected at any step.
    """

    validated = validate_assignments(assignments)
    if selected_joint is not None:
        matches = tuple(item for item in validated if item.joint_name == selected_joint)
        if not matches:
            choices = ", ".join(item.joint_name for item in validated)
            raise ValueError(f"Unknown joint '{selected_joint}'. Expected one of: {choices}")
        return matches
    return tuple(reversed(validated))


def provision_servos(
    bus: ProvisioningBus,
    plan: Iterable[ServoAssignment],
    *,
    confirm: Callable[[str], str] = input,
    output: Callable[[str], None] = print,
) -> tuple[ServoAssignment, ...]:
    """Interactively assign IDs, stopping before any unconfirmed write.

    LeRobot's ``setup_motor`` scans for the one attached STS3215, disables its
    torque, writes the target ID and standard baud rate, then changes the local
    bus to that baud rate.  This function supplies the physical safety gate
    that a serial protocol cannot enforce: exactly one servo must be attached.
    """

    completed: list[ServoAssignment] = []
    for assignment in validate_assignments(plan):
        prompt = (
            "\nTurn servo power OFF. Connect ONLY the servo labelled "
            f"'{assignment.joint_name}' (joint reference: '{assignment.joint_ref_name}').\n"
            "Turn servo power ON, keep the horn unloaded, then type PROGRAM "
            f"to write ID {assignment.servo_id}: "
        )
        if confirm(prompt).strip() != "PROGRAM":
            raise ProvisioningCancelled(
                f"Cancelled before writing '{assignment.joint_name}'. No write was attempted for this servo."
            )

        bus.setup_motor(assignment.joint_name)
        completed.append(assignment)
        output(
            f"Programmed {assignment.joint_name} as ID {assignment.servo_id}. "
            "Turn servo power OFF before disconnecting it, and attach its physical label now."
        )

    return tuple(completed)
