"""Check the joint-name contract between Orion's URDF and MJCF models."""

import importlib.util
import hashlib
import json
import math
from pathlib import Path
import xml.etree.ElementTree as ET


PACKAGE_ROOT = Path(__file__).resolve().parents[1]
PROJECT_ROOT = PACKAGE_ROOT.parents[2]
URDF_PATH = PACKAGE_ROOT / "urdf" / "orion.urdf"
MJCF_PATH = PROJECT_ROOT / "simulation" / "mujoco" / "robot.xml"
LAUNCH_PATH = PACKAGE_ROOT / "launch" / "mujoco.launch.py"
MUJOCO_CONFIG = PROJECT_ROOT / "simulation" / "mujoco" / "config"


def _load_mujoco_launch_module():
    spec = importlib.util.spec_from_file_location(
        "orion_mujoco_launch", LAUNCH_PATH
    )
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _ros_control_joint_names() -> tuple[str, ...]:
    root = ET.parse(URDF_PATH).getroot()
    ros2_control = root.find("ros2_control")
    assert ros2_control is not None
    return tuple(
        joint.get("name") for joint in ros2_control.findall("joint")
    )


def test_every_controlled_joint_has_a_mujoco_joint_and_position_actuator():
    mjcf_root = ET.parse(MJCF_PATH).getroot()
    mjcf_joint_names = {
        joint.get("name") for joint in mjcf_root.findall(".//joint")
    }
    position_actuators = {
        actuator.get("joint")
        for actuator in mjcf_root.findall("./actuator/position")
    }

    controlled_joint_names = _ros_control_joint_names()

    assert controlled_joint_names == (
        "base_yaw_joint",
        "shoulder_pitch_joint",
        "elbow_pitch_joint",
        "head_roll_joint",
        "head_pitch_joint",
    )
    assert set(controlled_joint_names) <= mjcf_joint_names
    assert set(controlled_joint_names) <= position_actuators


def test_scene_includes_the_canonical_robot_model():
    scene_path = MJCF_PATH.with_name("scene.xml")
    root = ET.parse(scene_path).getroot()
    include = root.find("include")

    assert include is not None
    assert include.get("file") == "robot.xml"


def test_mjcf_zero_and_ranges_match_the_accepted_physical_calibration():
    calibration_path = MUJOCO_CONFIG / "servo_calibration.json"
    reference_path = MUJOCO_CONFIG / "model_reference.json"
    calibration_bytes = calibration_path.read_bytes()
    calibration = json.loads(calibration_bytes)
    reference = json.loads(reference_path.read_text(encoding="utf-8"))
    assert hashlib.sha256(calibration_bytes).hexdigest() == reference[
        "source_calibration_sha256"
    ]

    resolution = calibration["encoder_resolution"]
    radians_per_count = 2.0 * math.pi / resolution
    mjcf_root = ET.parse(MJCF_PATH).getroot()
    mjcf_joints = {
        joint.get("name"): joint for joint in mjcf_root.findall(".//joint")
    }
    expected_references = reference["joint_reference_radians"]

    controlled_joints = _ros_control_joint_names()
    for joint_name in controlled_joints:
        physical = calibration["joints"][joint_name]
        direction = physical["encoder_direction"]
        expected_range = sorted(
            (
                physical["safe_min_delta_raw"] * radians_per_count / direction,
                physical["safe_max_delta_raw"] * radians_per_count / direction,
            )
        )
        mjcf_joint = mjcf_joints[joint_name]
        actual_range = [float(value) for value in mjcf_joint.get("range").split()]

        assert math.isclose(
            float(mjcf_joint.get("ref")),
            expected_references[joint_name],
            abs_tol=1e-14,
        )
        assert all(
            math.isclose(actual, expected, abs_tol=1e-14)
            for actual, expected in zip(actual_range, expected_range, strict=True)
        )

    zero_keyframe = mjcf_root.find("./keyframe/key[@name='zero_reference']")
    assert zero_keyframe is not None
    qpos = [float(value) for value in zero_keyframe.get("qpos").split()]
    assert qpos[7:] == [0.0] * len(controlled_joints)
    assert math.isclose(sum(value * value for value in qpos[3:7]), 1.0)


def test_launch_description_swaps_only_the_simulator_control_backend():
    launch_module = _load_mujoco_launch_module()
    scene_path = MJCF_PATH.with_name("scene.xml")

    description = launch_module.build_mujoco_robot_description(
        URDF_PATH,
        scene_path,
        headless=True,
    )
    root = ET.fromstring(description)
    ros2_control = root.find("ros2_control")
    assert ros2_control is not None
    hardware = ros2_control.find("hardware")
    assert hardware is not None

    assert ros2_control.get("name") == "OrionMujocoSystem"
    assert hardware.findtext("plugin") == (
        "mujoco_ros2_control/MujocoSystemInterface"
    )
    assert hardware.findtext("param[@name='mujoco_model']") == str(scene_path)
    assert hardware.findtext("param[@name='headless']") == "true"
    assert hardware.findtext("param[@name='initial_keyframe']") == "zero_reference"
    assert hardware.find("param[@name='odom_free_joint_name']") is None
    assert root.find("gazebo") is None
    assert tuple(
        joint.get("name") for joint in ros2_control.findall("joint")
    ) == _ros_control_joint_names()
