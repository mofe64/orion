"""Acquire complete measured Orion joint state from ROS."""

from __future__ import annotations

import math
import time
from dataclasses import dataclass
from typing import Sequence

import rclpy
from rclpy.node import Node
from rclpy.qos import qos_profile_sensor_data
from sensor_msgs.msg import JointState


JOINT_STATE_TOPIC = "/joint_states"


class JointStateError(ValueError):
    """Raised when ROS joint feedback is incomplete or malformed."""


@dataclass(frozen=True)
class MeasuredJointState:
    """Position and velocity aligned to Orion's canonical joint order."""

    positions: tuple[float, ...]
    velocities: tuple[float, ...]
    received_at: float | None = None

    def age(self, *, now: float | None = None) -> float:
        """Return monotonic sample age, or infinity when receipt is unknown."""

        if self.received_at is None:
            return math.inf
        current = time.monotonic() if now is None else now
        return max(0.0, current - self.received_at)


def joint_state_to_measured_state(
    message: JointState,
    joint_names: Sequence[str],
    *,
    received_at: float | None = None,
) -> MeasuredJointState:
    """Extract a complete canonical state from one ROS JointState message."""

    if len(set(message.name)) != len(message.name):
        raise JointStateError("joint state contains duplicate joint names")
    if len(message.position) != len(message.name):
        raise JointStateError("joint state positions must match its names")
    if len(message.velocity) != len(message.name):
        raise JointStateError(
            "joint state velocities are required and must match its names"
        )

    index_by_name = {name: index for index, name in enumerate(message.name)}
    missing = [name for name in joint_names if name not in index_by_name]
    if missing:
        raise JointStateError(
            f"joint state is missing required joints: {missing}"
        )

    return MeasuredJointState(
        positions=tuple(
            message.position[index_by_name[name]] for name in joint_names
        ),
        velocities=tuple(
            message.velocity[index_by_name[name]] for name in joint_names
        ),
        received_at=(
            time.monotonic() if received_at is None else received_at
        ),
    )


def require_fresh_measured_state(
    state: MeasuredJointState,
    maximum_age: float,
    *,
    now: float | None = None,
) -> None:
    """Reject state too old to remain a safe trajectory start condition."""

    if not math.isfinite(maximum_age) or maximum_age <= 0:
        raise ValueError("maximum state age must be finite and positive")
    age = state.age(now=now)
    if age > maximum_age:
        raise JointStateError(
            f"measured joint state age {age:.3f} seconds exceeds "
            f"maximum {maximum_age:.3f} seconds"
        )


def wait_for_measured_joint_state(
    node: Node,
    joint_names: Sequence[str],
    *,
    timeout: float,
) -> MeasuredJointState:
    """Wait for a newly delivered complete joint-state sample."""

    received: list[tuple[JointState, float]] = []

    def receive(message: JointState) -> None:
        received.append((message, time.monotonic()))

    subscription = node.create_subscription(
        JointState,
        JOINT_STATE_TOPIC,
        receive,
        qos_profile_sensor_data,
    )
    deadline = time.monotonic() + timeout
    try:
        while not received:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise JointStateError(
                    f"no joint state received from {JOINT_STATE_TOPIC} within "
                    f"{timeout:.1f} seconds"
                )
            rclpy.spin_once(node, timeout_sec=min(0.1, remaining))
        message, received_at = received[-1]
        return joint_state_to_measured_state(
            message,
            joint_names,
            received_at=received_at,
        )
    finally:
        node.destroy_subscription(subscription)
