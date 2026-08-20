"""Tests for Gazebo base stability measured through ROS topics."""

from copy import deepcopy
from pathlib import Path

import pytest
from nav_msgs.msg import Odometry
from ros_gz_interfaces.msg import Contact, Contacts

from orion_motion.execution_types import ExecutionResult, ExecutionStatus
from orion_motion.motion_loader import load_yaml_file
from orion_motion.ros_stability_monitor import (
    RosBaseStabilityMonitor,
    ros_base_stability_policy_from_data,
)


CONFIG_DIRECTORY = Path(__file__).parent.parent / "config"


class FakeNode:
    def __init__(self):
        self.callbacks = {}
        self.destroyed = []

    def create_subscription(self, message_type, topic, callback, qos):
        subscription = object()
        self.callbacks[topic] = callback
        return subscription

    def destroy_subscription(self, subscription):
        self.destroyed.append(subscription)


def make_odometry(x=0.0, y=0.0, z=0.0, *, tilt=0.0):
    message = Odometry()
    message.pose.pose.position.x = x
    message.pose.pose.position.y = y
    message.pose.pose.position.z = z
    message.pose.pose.orientation.x = 0.0
    message.pose.pose.orientation.y = 0.0
    message.pose.pose.orientation.z = 0.0
    message.pose.pose.orientation.w = 1.0
    if tilt:
        from math import cos, sin

        message.pose.pose.orientation.x = sin(tilt / 2.0)
        message.pose.pose.orientation.w = cos(tilt / 2.0)
    return message


def make_contacts(timestamp, *, supported):
    message = Contacts()
    message.header.stamp.sec = int(timestamp)
    message.header.stamp.nanosec = round(
        (timestamp - int(timestamp)) * 1_000_000_000
    )
    if supported:
        contact = Contact()
        contact.collision1.name = "orion::base_support_collision"
        contact.collision2.name = "ground_plane::link::collision"
        message.contacts.append(contact)
    return message


def project_policy():
    return ros_base_stability_policy_from_data(
        load_yaml_file(CONFIG_DIRECTORY / "stability_limits.yaml")
    )


def test_monitor_merges_safe_pose_and_contact_metrics_into_result():
    node = FakeNode()
    monitor = RosBaseStabilityMonitor(node, project_policy())
    monitor.receive_odometry(make_odometry())
    monitor.receive_contacts(make_contacts(10.0, supported=True))

    monitor.begin()
    monitor.receive_odometry(make_odometry(0.002, 0.0, 0.001, tilt=0.02))
    monitor.receive_contacts(make_contacts(10.02, supported=False))
    monitor.receive_contacts(make_contacts(10.04, supported=True))
    result = monitor.enrich_result(
        ExecutionResult(
            motion_name="look_at_left",
            backend="ros2_control",
            status=ExecutionStatus.SUCCEEDED,
            message="done",
        )
    )

    assert result.status is ExecutionStatus.SUCCEEDED
    assert result.metrics.maximum_base_translation == pytest.approx(
        (0.002**2 + 0.001**2) ** 0.5
    )
    assert result.metrics.maximum_base_tilt == pytest.approx(0.02)
    assert result.metrics.maximum_base_height_change == pytest.approx(0.001)
    assert result.metrics.longest_contact_loss == pytest.approx(0.02)


def test_missing_measurements_turn_success_into_unsafe_stability():
    monitor = RosBaseStabilityMonitor(FakeNode(), project_policy())
    monitor.begin()

    result = monitor.enrich_result(
        ExecutionResult(
            motion_name="look_at_left",
            backend="ros2_control",
            status=ExecutionStatus.SUCCEEDED,
            message="done",
        )
    )

    assert result.status is ExecutionStatus.UNSAFE_STABILITY
    assert "no base odometry" in result.message
    assert "no base contact" in result.message


@pytest.mark.parametrize(
    "section,field",
    [
        ("base", "maximum_translation"),
        ("base", "maximum_tilt"),
        ("base", "maximum_height_change"),
        ("contact", "maximum_loss_duration"),
    ],
)
def test_policy_requires_positive_finite_bounds(section, field):
    data = deepcopy(load_yaml_file(CONFIG_DIRECTORY / "stability_limits.yaml"))
    data[section][field] = 0.0

    with pytest.raises(ValueError, match=field):
        ros_base_stability_policy_from_data(data)


def test_monitor_destroys_both_subscriptions():
    node = FakeNode()
    monitor = RosBaseStabilityMonitor(node, project_policy())

    monitor.close()

    assert len(node.destroyed) == 2
