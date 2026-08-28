"""Construction of Orion's LeRobot-backed STS3215 bus."""

from __future__ import annotations

from collections.abc import Sequence

from .provisioning import ServoAssignment


def create_lerobot_bus(port: str, assignments: Sequence[ServoAssignment]):
    """Create the hardware bus while keeping LeRobot optional for dry runs."""

    try:
        from lerobot.motors import Motor, MotorNormMode
        from lerobot.motors.feetech import FeetechMotorsBus
    except ImportError as exc:
        raise RuntimeError(
            "LeRobot's Feetech support is unavailable. Run `uv sync` in hardware/servo_setup first."
        ) from exc

    motors = {
        assignment.joint_name: Motor(
            id=assignment.servo_id,
            model="sts3215",
            norm_mode=MotorNormMode.RANGE_M100_100,
        )
        for assignment in assignments
    }
    return FeetechMotorsBus(port=port, motors=motors)
