"""Regression tests for Orion's ROS-independent robot descriptions."""

import hashlib
import json
import math
from pathlib import Path
import xml.etree.ElementTree as ET


PROJECT_ROOT = Path(__file__).resolve().parents[1]
URDF_PATH = PROJECT_ROOT / "description" / "urdf" / "orion.urdf"
MJCF_PATH = PROJECT_ROOT / "simulation" / "mujoco" / "robot.xml"
MUJOCO_CONFIG = PROJECT_ROOT / "simulation" / "mujoco" / "config"
CANONICAL_JOINTS = (
    "base_yaw_joint",
    "shoulder_pitch_joint",
    "elbow_pitch_joint",
    "head_roll_joint",
    "head_pitch_joint",
)


def test_urdf_is_backend_neutral_and_all_meshes_resolve():
    root = ET.parse(URDF_PATH).getroot()

    assert root.find("ros2_control") is None
    assert root.find("gazebo") is None
    for mesh in root.findall(".//mesh"):
        filename = mesh.get("filename")
        assert filename is not None
        assert not filename.startswith("package://")
        assert (URDF_PATH.parent / filename).resolve().is_file()


def test_urdf_and_mujoco_share_the_canonical_joint_contract():
    urdf_root = ET.parse(URDF_PATH).getroot()
    mjcf_root = ET.parse(MJCF_PATH).getroot()

    urdf_names = tuple(
        name
        for name in CANONICAL_JOINTS
        if urdf_root.find(f"joint[@name='{name}']") is not None
    )
    mjcf_joint_names = {
        joint.get("name") for joint in mjcf_root.findall(".//joint")
    }
    position_actuators = {
        actuator.get("joint")
        for actuator in mjcf_root.findall("./actuator/position")
    }

    assert urdf_names == CANONICAL_JOINTS
    assert set(CANONICAL_JOINTS) <= mjcf_joint_names
    assert set(CANONICAL_JOINTS) <= position_actuators


def test_mujoco_uses_the_shared_mesh_directory():
    root = ET.parse(MJCF_PATH).getroot()
    compiler = root.find("compiler")
    assert compiler is not None
    mesh_directory = (MJCF_PATH.parent / compiler.get("meshdir")).resolve()
    assert mesh_directory == (PROJECT_ROOT / "description" / "meshes").resolve()
    for mesh in root.findall("./asset/mesh"):
        assert (mesh_directory / mesh.get("file")).is_file()


def test_mujoco_zero_and_ranges_match_the_accepted_physical_calibration():
    calibration_path = MUJOCO_CONFIG / "servo_calibration.json"
    reference_path = MUJOCO_CONFIG / "model_reference.json"
    calibration_bytes = calibration_path.read_bytes()
    calibration = json.loads(calibration_bytes)
    reference = json.loads(reference_path.read_text(encoding="utf-8"))
    assert hashlib.sha256(calibration_bytes).hexdigest() == reference[
        "source_calibration_sha256"
    ]

    radians_per_count = 2.0 * math.pi / calibration["encoder_resolution"]
    mjcf_root = ET.parse(MJCF_PATH).getroot()
    mjcf_joints = {
        joint.get("name"): joint for joint in mjcf_root.findall(".//joint")
    }
    expected_references = reference["joint_reference_radians"]

    for joint_name in CANONICAL_JOINTS:
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

    zero = mjcf_root.find("./keyframe/key[@name='zero_reference']")
    assert zero is not None
    qpos = [float(value) for value in zero.get("qpos").split()]
    assert qpos[7:] == [0.0] * len(CANONICAL_JOINTS)
    assert math.isclose(sum(value * value for value in qpos[3:7]), 1.0)
