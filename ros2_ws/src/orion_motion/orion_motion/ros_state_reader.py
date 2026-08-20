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

    for field_name, values in (
        ("position", message.position),
        ("velocity", message.velocity),
    ):
        invalid = [
            name
            for name in joint_names
            if not math.isfinite(values[index_by_name[name]])
        ]
        if invalid:
            raise JointStateError(
                f"joint state has non-finite {field_name} values for: {invalid}"
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


def wait_for_stopped_joint_state(
    node: Node,
    joint_names: Sequence[str],
    *,
    maximum_velocity: float,
    stable_duration: float,
    timeout: float,
) -> MeasuredJointState:
    """Wait until fresh feedback remains stopped for a bounded duration."""

    for field_name, value in (
        ("maximum_velocity", maximum_velocity),
        ("stable_duration", stable_duration),
        ("timeout", timeout),
    ):
        if not math.isfinite(value) or value <= 0:
            raise ValueError(f"{field_name} must be finite and positive")

    stopped_since: float | None = None
    latest_stopped: MeasuredJointState | None = None
    last_error: JointStateError | None = None

    def receive(message: JointState) -> None:
        nonlocal stopped_since, latest_stopped, last_error
        received_at = time.monotonic()
        try:
            measured = joint_state_to_measured_state(
                message,
                joint_names,
                received_at=received_at,
            )
        except JointStateError as error:
            last_error = error
            stopped_since = None
            latest_stopped = None
            return

        if all(
            abs(velocity) <= maximum_velocity
            for velocity in measured.velocities
        ):
            if stopped_since is None:
                stopped_since = received_at
            latest_stopped = measured
        else:
            stopped_since = None
            latest_stopped = None

    subscription = node.create_subscription(
        JointState,
        JOINT_STATE_TOPIC,
        receive,
        qos_profile_sensor_data,
    )
    deadline = time.monotonic() + timeout
    try:
        while True:
            now = time.monotonic()
            if (
                stopped_since is not None
                and latest_stopped is not None
                and now - stopped_since >= stable_duration
            ):
                return latest_stopped
            remaining = deadline - now
            if remaining <= 0:
                detail = f" Last invalid sample: {last_error}" if last_error else ""
                raise JointStateError(
                    "joints did not remain below "
                    f"{maximum_velocity:.3f} rad/s for "
                    f"{stable_duration:.3f} seconds within "
                    f"{timeout:.3f} seconds.{detail}"
                )
            rclpy.spin_once(node, timeout_sec=min(0.05, remaining))
    finally:
        node.destroy_subscription(subscription)


def wait_for_settled_joint_state(
    node: Node,
    joint_names: Sequence[str],
    target_positions: Sequence[float],
    *,
    maximum_position_error: float,
    maximum_velocity: float,
    stable_duration: float,
    timeout: float,
) -> MeasuredJointState:
    """Wait until the final target remains reached and stopped."""

    if len(target_positions) != len(joint_names):
        raise ValueError("target_positions must match joint_names")
    if any(not math.isfinite(position) for position in target_positions):
        raise ValueError("target_positions must contain only finite values")
    for field_name, value in (
        ("maximum_position_error", maximum_position_error),
        ("maximum_velocity", maximum_velocity),
        ("stable_duration", stable_duration),
        ("timeout", timeout),
    ):
        if not math.isfinite(value) or value <= 0:
            raise ValueError(f"{field_name} must be finite and positive")

    settled_since: float | None = None
    latest_settled: MeasuredJointState | None = None
    last_error: JointStateError | None = None

    def receive(message: JointState) -> None:
        nonlocal settled_since, latest_settled, last_error
        received_at = time.monotonic()
        try:
            measured = joint_state_to_measured_state(
                message,
                joint_names,
                received_at=received_at,
            )
        except JointStateError as error:
            last_error = error
            settled_since = None
            latest_settled = None
            return

        positions_reached = all(
            abs(target - actual) <= maximum_position_error
            for target, actual in zip(
                target_positions,
                measured.positions,
                strict=True,
            )
        )
        stopped = all(
            abs(velocity) <= maximum_velocity
            for velocity in measured.velocities
        )
        if positions_reached and stopped:
            if settled_since is None:
                settled_since = received_at
            latest_settled = measured
        else:
            settled_since = None
            latest_settled = None

    subscription = node.create_subscription(
        JointState,
        JOINT_STATE_TOPIC,
        receive,
        qos_profile_sensor_data,
    )
    deadline = time.monotonic() + timeout
    try:
        while True:
            now = time.monotonic()
            if (
                settled_since is not None
                and latest_settled is not None
                and now - settled_since >= stable_duration
            ):
                return latest_settled
            remaining = deadline - now
            if remaining <= 0:
                detail = f" Last invalid sample: {last_error}" if last_error else ""
                raise JointStateError(
                    "joints did not remain within "
                    f"{maximum_position_error:.3f} rad of the final target and "
                    f"below {maximum_velocity:.3f} rad/s for "
                    f"{stable_duration:.3f} seconds within "
                    f"{timeout:.3f} seconds.{detail}"
                )
            rclpy.spin_once(node, timeout_sec=min(0.05, remaining))
    finally:
        node.destroy_subscription(subscription)
