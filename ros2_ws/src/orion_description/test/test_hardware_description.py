"""Check the physical ros2_control description generated at launch time."""

import importlib.util
from pathlib import Path
import xml.etree.ElementTree as ET


PACKAGE_ROOT = Path(__file__).resolve().parents[1]
URDF_PATH = PACKAGE_ROOT / "urdf" / "orion.urdf"
LAUNCH_PATH = PACKAGE_ROOT / "launch" / "hardware.launch.py"


def _load_hardware_launch_module():
    spec = importlib.util.spec_from_file_location(
        "orion_hardware_launch", LAUNCH_PATH
    )
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_launch_description_selects_the_physical_plugin_and_parameters():
    launch_module = _load_hardware_launch_module()
    calibration = Path("/tmp/orion-calibration.json")

    description = launch_module.build_hardware_robot_description(
        URDF_PATH,
        port="/dev/ttyFAKE0",
        baud_rate=1_000_000,
        calibration_file=calibration,
    )
    root = ET.fromstring(description)
    ros2_control = root.find("ros2_control")
    assert ros2_control is not None
    hardware = ros2_control.find("hardware")
    assert hardware is not None

    assert ros2_control.get("name") == "OrionSTS3215System"
    assert hardware.findtext("plugin") == "orion_hardware/STS3215System"
    assert hardware.findtext("param[@name='port']") == "/dev/ttyFAKE0"
    assert hardware.findtext("param[@name='baud_rate']") == "1000000"
    assert hardware.findtext("param[@name='calibration_file']") == str(
        calibration
    )
    assert root.find("gazebo") is None


def test_physical_interfaces_preserve_joint_order_without_fake_effort():
    launch_module = _load_hardware_launch_module()
    description = launch_module.build_hardware_robot_description(
        URDF_PATH,
        port="/dev/ttyFAKE0",
        baud_rate=1_000_000,
        calibration_file=Path("/tmp/orion-calibration.json"),
    )
    root = ET.fromstring(description)
    ros2_control = root.find("ros2_control")
    assert ros2_control is not None
    joints = ros2_control.findall("joint")

    assert tuple(joint.get("name") for joint in joints) == (
        "base_yaw_joint",
        "shoulder_pitch_joint",
        "elbow_pitch_joint",
        "head_roll_joint",
        "head_pitch_joint",
    )
    for joint in joints:
        assert [
            interface.get("name")
            for interface in joint.findall("command_interface")
        ] == ["position"]
        assert [
            interface.get("name")
            for interface in joint.findall("state_interface")
        ] == ["position", "velocity"]
