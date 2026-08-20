"""Measure Gazebo base movement and floor contact from ROS topics."""

from __future__ import annotations

from dataclasses import dataclass, replace
import math
import time
from typing import Any

from nav_msgs.msg import Odometry
from rclpy.node import Node
from rclpy.qos import qos_profile_sensor_data
from ros_gz_interfaces.msg import Contacts

from orion_motion.execution_types import (
    ExecutionMetrics,
    ExecutionResult,
    ExecutionStatus,
)


BASE_ODOMETRY_TOPIC = "/orion/base_odometry"
BASE_CONTACT_TOPIC = "/orion/base_contacts"


@dataclass(frozen=True)
class RosBaseStabilityPolicy:
    """Bounds and names needed by the ROS base monitor."""

    maximum_translation: float
    maximum_tilt: float
    maximum_height_change: float
    maximum_contact_loss_duration: float
    base_collision_name: str = "base_support_collision"
    floor_collision_name: str = "ground_plane"


@dataclass(frozen=True)
class RosBaseStabilitySnapshot:
    """Accumulated measurements from one ROS-controlled motion."""

    maximum_translation: float | None
    maximum_tilt: float | None
    maximum_height_change: float | None
    longest_contact_loss: float | None
    unsafe_reasons: tuple[str, ...]

    @property
    def safe(self) -> bool:
        return not self.unsafe_reasons


def ros_base_stability_policy_from_data(data: Any) -> RosBaseStabilityPolicy:
    """Load the base and contact bounds from shared stability data."""

    if not isinstance(data, dict) or data.get("format_version") != 1:
        raise ValueError("stability_limits.format_version must be 1")
    base = data.get("base")
    contact = data.get("contact")
    if not isinstance(base, dict) or not isinstance(contact, dict):
        raise ValueError("stability limits require base and contact mappings")
    values = {
        "base.maximum_translation": base.get("maximum_translation"),
        "base.maximum_tilt": base.get("maximum_tilt"),
        "base.maximum_height_change": base.get("maximum_height_change"),
        "contact.maximum_loss_duration": contact.get(
            "maximum_loss_duration"
        ),
    }
    for name, value in values.items():
        if (
            isinstance(value, bool)
            or not isinstance(value, (int, float))
            or not math.isfinite(value)
            or value <= 0
        ):
            raise ValueError(f"{name} must be finite and positive")
    return RosBaseStabilityPolicy(
        maximum_translation=float(base["maximum_translation"]),
        maximum_tilt=float(base["maximum_tilt"]),
        maximum_height_change=float(base["maximum_height_change"]),
        maximum_contact_loss_duration=float(
            contact["maximum_loss_duration"]
        ),
    )


def _stamp_seconds(stamp: Any) -> float:
    value = float(stamp.sec) + float(stamp.nanosec) / 1_000_000_000
    return value if value > 0.0 else time.monotonic()


def _up_vector(orientation: Any) -> tuple[float, float, float]:
    x = float(orientation.x)
    y = float(orientation.y)
    z = float(orientation.z)
    w = float(orientation.w)
    norm = math.sqrt(x * x + y * y + z * z + w * w)
    if not math.isfinite(norm) or norm <= 0.0:
        raise ValueError("base odometry orientation must be a finite quaternion")
    x, y, z, w = (value / norm for value in (x, y, z, w))
    return (
        2.0 * (x * z + w * y),
        2.0 * (y * z - w * x),
        1.0 - 2.0 * (x * x + y * y),
    )


def _has_floor_support(message: Contacts, policy: RosBaseStabilityPolicy) -> bool:
    for contact in message.contacts:
        first = contact.collision1.name
        second = contact.collision2.name
        if (
            policy.base_collision_name in first
            and policy.floor_collision_name in second
        ) or (
            policy.base_collision_name in second
            and policy.floor_collision_name in first
        ):
            return True
    return False


class RosBaseStabilityMonitor:
    """Accumulate Gazebo base pose and contact evidence for each goal."""

    def __init__(self, node: Node, policy: RosBaseStabilityPolicy) -> None:
        self._node = node
        self._policy = policy
        self._latest_pose: tuple[
            tuple[float, float, float], tuple[float, float, float]
        ] | None = None
        self._latest_contact: tuple[float, bool] | None = None
        self._active = False
        self._reference_position: tuple[float, float, float] | None = None
        self._reference_up: tuple[float, float, float] | None = None
        self._maximum_translation = 0.0
        self._maximum_tilt = 0.0
        self._maximum_height_change = 0.0
        self._current_contact_loss = 0.0
        self._longest_contact_loss = 0.0
        self._pose_samples = 0
        self._contact_samples = 0
        self._odometry_subscription = node.create_subscription(
            Odometry,
            BASE_ODOMETRY_TOPIC,
            self.receive_odometry,
            qos_profile_sensor_data,
        )
        self._contact_subscription = node.create_subscription(
            Contacts,
            BASE_CONTACT_TOPIC,
            self.receive_contacts,
            qos_profile_sensor_data,
        )

    def begin(self) -> None:
        """Reset measurements using the newest base state as the reference."""

        self._active = True
        self._reference_position = (
            self._latest_pose[0] if self._latest_pose is not None else None
        )
        self._reference_up = (
            self._latest_pose[1] if self._latest_pose is not None else None
        )
        self._maximum_translation = 0.0
        self._maximum_tilt = 0.0
        self._maximum_height_change = 0.0
        self._current_contact_loss = 0.0
        self._longest_contact_loss = 0.0
        self._pose_samples = 0
        self._contact_samples = 0

    def receive_odometry(self, message: Odometry) -> None:
        """Record one measured world pose of `base_footprint`."""

        position_message = message.pose.pose.position
        position = (
            float(position_message.x),
            float(position_message.y),
            float(position_message.z),
        )
        if any(not math.isfinite(value) for value in position):
            return
        try:
            up = _up_vector(message.pose.pose.orientation)
        except ValueError:
            return
        self._latest_pose = (position, up)
        if not self._active:
            return
        if self._reference_position is None or self._reference_up is None:
            self._reference_position = position
            self._reference_up = up
        difference = tuple(
            actual - reference
            for actual, reference in zip(
                position, self._reference_position, strict=True
            )
        )
        translation = math.sqrt(sum(value * value for value in difference))
        up_dot = sum(
            first * second
            for first, second in zip(up, self._reference_up, strict=True)
        )
        tilt = math.acos(max(-1.0, min(1.0, up_dot)))
        self._maximum_translation = max(
            self._maximum_translation, translation
        )
        self._maximum_tilt = max(self._maximum_tilt, tilt)
        self._maximum_height_change = max(
            self._maximum_height_change, abs(difference[2])
        )
        self._pose_samples += 1

    def receive_contacts(self, message: Contacts) -> None:
        """Record whether the named support box still touches the floor."""

        timestamp = _stamp_seconds(message.header.stamp)
        supported = _has_floor_support(message, self._policy)
        previous = self._latest_contact
        self._latest_contact = (timestamp, supported)
        if not self._active:
            return
        elapsed = 0.0
        if previous is not None:
            elapsed = max(0.0, timestamp - previous[0])
        if supported:
            self._current_contact_loss = 0.0
        else:
            self._current_contact_loss += elapsed
            self._longest_contact_loss = max(
                self._longest_contact_loss,
                self._current_contact_loss,
            )
        self._contact_samples += 1

    def snapshot(self) -> RosBaseStabilitySnapshot:
        """Return current measurements and threshold failures."""

        reasons: list[str] = []
        if self._pose_samples == 0:
            reasons.append("no base odometry was measured during the motion")
        if self._contact_samples == 0:
            reasons.append("no base contact was measured during the motion")
        if self._maximum_translation > self._policy.maximum_translation:
            reasons.append("base translation exceeded its limit")
        if self._maximum_tilt > self._policy.maximum_tilt:
            reasons.append("base tilt exceeded its limit")
        if self._maximum_height_change > self._policy.maximum_height_change:
            reasons.append("base height change exceeded its limit")
        if (
            self._longest_contact_loss
            > self._policy.maximum_contact_loss_duration
        ):
            reasons.append("base lost floor contact for too long")
        return RosBaseStabilitySnapshot(
            maximum_translation=(
                self._maximum_translation if self._pose_samples else None
            ),
            maximum_tilt=(self._maximum_tilt if self._pose_samples else None),
            maximum_height_change=(
                self._maximum_height_change if self._pose_samples else None
            ),
            longest_contact_loss=(
                self._longest_contact_loss if self._contact_samples else None
            ),
            unsafe_reasons=tuple(reasons),
        )

    def enrich_result(self, result: ExecutionResult) -> ExecutionResult:
        """Merge base evidence into a result and reject unsafe success."""

        self._active = False
        snapshot = self.snapshot()
        metrics = result.metrics or ExecutionMetrics()
        enriched = replace(
            result,
            metrics=replace(
                metrics,
                maximum_base_translation=snapshot.maximum_translation,
                maximum_base_tilt=snapshot.maximum_tilt,
                maximum_base_height_change=snapshot.maximum_height_change,
                longest_contact_loss=snapshot.longest_contact_loss,
            ),
        )
        if enriched.succeeded and not snapshot.safe:
            return replace(
                enriched,
                status=ExecutionStatus.UNSAFE_STABILITY,
                message=(
                    f"{enriched.message}; stability failed: "
                    + "; ".join(snapshot.unsafe_reasons)
                ),
            )
        return enriched

    def close(self) -> None:
        """Destroy both subscriptions owned by this monitor."""

        self._node.destroy_subscription(self._odometry_subscription)
        self._node.destroy_subscription(self._contact_subscription)
