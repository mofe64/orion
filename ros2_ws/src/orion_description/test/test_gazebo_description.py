"""Check Orion's explicit Gazebo base-odometry contract."""

import importlib.util
from pathlib import Path
import xml.etree.ElementTree as ET


PACKAGE_ROOT = Path(__file__).resolve().parents[1]
URDF_PATH = PACKAGE_ROOT / "urdf" / "orion.urdf"
LAUNCH_PATH = PACKAGE_ROOT / "launch" / "gazebo.launch.py"


def _load_gazebo_launch_module():
    spec = importlib.util.spec_from_file_location(
        "orion_gazebo_launch", LAUNCH_PATH
    )
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_gazebo_publishes_named_three_dimensional_base_odometry():
    root = ET.parse(URDF_PATH).getroot()
    plugin = root.find(
        "gazebo/plugin[@name='gz::sim::systems::OdometryPublisher']"
    )

    assert plugin is not None
    assert plugin.get("filename") == "gz-sim-odometry-publisher-system"
    assert plugin.findtext("odom_frame") == "world"
    assert plugin.findtext("robot_base_frame") == "base_footprint"
    assert plugin.findtext("child_frame_id") == "base_footprint"
    assert plugin.findtext("dimensions") == "3"
    assert plugin.findtext("odom_topic") == "/orion/base_odometry"
    assert plugin.findtext("odom_publish_frequency") == "100"


def test_gazebo_base_uses_the_shared_support_shape_and_contact_sensor():
    root = ET.parse(URDF_PATH).getroot()
    support = root.find(
        "link[@name='base_link']/collision[@name='base_support_collision']"
    )
    sensor = root.find(
        "gazebo[@reference='base_link']/sensor[@name='base_contact_sensor']"
    )

    assert support is not None
    assert support.find("geometry/box").get("size") == (
        "0.225084 0.205614 0.04"
    )
    assert sensor is not None
    assert sensor.get("type") == "contact"
    assert sensor.findtext("topic") == "/orion/base_contacts"
    assert sensor.findtext("contact/collision") == (
        "base_footprint_fixed_joint_lump__base_support_collision_collision_10"
    )


def test_gazebo_odometry_is_bridged_one_way_into_ros():
    launch_module = _load_gazebo_launch_module()

    assert launch_module.BASE_ODOMETRY_TOPIC == "/orion/base_odometry"
    assert launch_module.BASE_ODOMETRY_BRIDGE == (
        "/orion/base_odometry@nav_msgs/msg/Odometry[gz.msgs.Odometry"
    )
    assert launch_module.BASE_CONTACT_BRIDGE == (
        "/world/empty/model/orion/link/base_footprint/sensor/"
        "base_contact_sensor/contact@ros_gz_interfaces/msg/Contacts"
        "[gz.msgs.Contacts"
    )
