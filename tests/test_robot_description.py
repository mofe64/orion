"""Regression tests for Orion's ROS-independent robot descriptions."""

import hashlib
import json
import math
from pathlib import Path
import runpy
import unittest
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


class CameraFrameDescriptionTests(unittest.TestCase):
    """Frame conventions and fixture provenance need only the Python stdlib."""

    def test_v2_models_use_calibrated_zero_and_exact_physical_limits(self):
        calibration_bytes = (MUJOCO_CONFIG / "servo_calibration-v2.json").read_bytes()
        calibration = json.loads(calibration_bytes)
        reference = json.loads((MUJOCO_CONFIG / "model_reference-v2.json").read_text())
        self.assertEqual(hashlib.sha256(calibration_bytes).hexdigest(), reference["source_calibration_sha256"])
        self.assertEqual(reference["applied_to"], [
            "description/urdf/orion-v2.urdf", "simulation/mujoco/v2/robot.xml",
        ])
        urdf = ET.parse(PROJECT_ROOT / reference["applied_to"][0]).getroot()
        mjcf = ET.parse(PROJECT_ROOT / reference["applied_to"][1]).getroot()
        for name in CANONICAL_JOINTS:
            with self.subTest(joint=name):
                physical = calibration["joints"][name]
                radians_per_count = 2 * math.pi / calibration["encoder_resolution"]
                limits = sorted(physical[key] * radians_per_count / physical["encoder_direction"]
                                for key in ("safe_min_delta_raw", "safe_max_delta_raw"))
                joint = urdf.find(f"joint[@name='{name}']")
                self.assertEqual([float(joint.find("limit").get(key)) for key in ("lower", "upper")], limits)
                mj_joint = mjcf.find(f".//joint[@name='{name}']")
                self.assertEqual([float(value) for value in mj_joint.get("range").split()], limits)
                self.assertEqual(float(mj_joint.get("ref")), reference["joint_reference_radians"][name])
                expected_roll = -reference["joint_reference_radians"][name] if name in (
                    "shoulder_pitch_joint", "elbow_pitch_joint", "head_pitch_joint",
                ) else 0.0
                self.assertEqual([float(value) for value in joint.find("origin").get("rpy").split()],
                                 [expected_roll, 0.0, 0.0])

    def test_v2_camera_and_light_are_fixed_nominal_neck_mounts(self):
        root = ET.parse(PROJECT_ROOT / "description/urdf/orion-v2.urdf").getroot()
        for joint_name, frame, position in (
            ("camera_optical_joint", "camera_optical_frame", [0.0, -0.082, 0.0157]),
            ("light_joint", "light_frame", [0.0, -0.077, 0.020]),
        ):
            with self.subTest(frame=frame):
                self.assertIsNotNone(root.find(f"link[@name='{frame}']"))
                joint = root.find(f"joint[@name='{joint_name}']")
                self.assertIsNotNone(joint)
                self.assertEqual(joint.get("type"), "fixed")
                self.assertEqual(joint.find("parent").get("link"), "v21_neck_swivel_link")
                self.assertEqual(joint.find("child").get("link"), frame)
                self.assertEqual([float(v) for v in joint.find("origin").get("xyz").split()], position)
                self.assertEqual(
                    [float(v) for v in joint.find("origin").get("rpy").split()],
                    [-math.pi / 2, 0.0, math.pi],
                )
        self.assertEqual(
            {joint.get("name") for joint in root.findall("joint") if joint.get("type") != "fixed"},
            set(CANONICAL_JOINTS),
        )

    def test_mujoco_mounts_account_for_its_camera_axis_convention(self):
        root = ET.parse(PROJECT_ROOT / "simulation/mujoco/v2/robot.xml").getroot()
        neck = root.find(".//body[@name='v21_neck_swivel_link']")
        camera = neck.find("camera[@name='camera_optical_frame']")
        light = neck.find("site[@name='light_frame']")
        self.assertIsNotNone(camera)
        self.assertIsNotNone(light)
        self.assertEqual(camera.get("mode"), "fixed")
        self.assertEqual([float(v) for v in camera.get("pos").split()], [0.0, -0.082, 0.0157])
        self.assertEqual([float(v) for v in light.get("pos").split()], [0.0, -0.077, 0.020])
        # Quaternion x=0, w=0: camera has y=z, light has y=-z.
        # These differ by 180 degrees about local X (flip up/back to down/forward).
        a = 1.0 / math.sqrt(2)
        for node, expected in ((camera, [0.0, 0.0, a, a]), (light, [0.0, 0.0, -a, a])):
            actual = [float(v) for v in node.get("quat").split()]
            for value, target in zip(actual, expected, strict=True):
                self.assertAlmostEqual(value, target, places=14)

    def test_camera_fk_fixture_is_complete_and_matches_its_sources(self):
        fixture = json.loads((PROJECT_ROOT / "runtime/tests/fixtures/camera_fk_v2.json").read_text())
        self.assertEqual(fixture["format_version"], 1)
        self.assertEqual(fixture["seed"], 2101)
        self.assertEqual(fixture["random_count"], 20)
        self.assertEqual(len(fixture["cases"]), 24)
        self.assertEqual(
            [case["name"] for case in fixture["cases"]],
            ["zero", "home", "look_left", "look_right"] + [f"random_{i:02d}" for i in range(20)],
        )
        requirements = (PROJECT_ROOT / "simulation/mujoco/requirements.txt").read_text()
        self.assertIn(f"mujoco=={fixture['mujoco_version']}", requirements.splitlines())
        for path, expected in fixture["source_sha256"].items():
            self.assertEqual(hashlib.sha256((PROJECT_ROOT / path).read_bytes()).hexdigest(), expected, path)
        for case in fixture["cases"]:
            self.assertEqual(set(case["positions"]), set(CANONICAL_JOINTS))
            for joint, value in case["positions"].items():
                lower, upper = fixture["model_joint_limits"][joint]
                self.assertLessEqual(lower, value)
                self.assertLessEqual(value, upper)


def test_camera_fk_fixture_reproduces_the_pinned_mujoco_results():
    # Pytest in the existing MuJoCo environment runs this independent numerical
    # check. Stdlib unittest and cargo test consume the fixture without MuJoCo.
    generator = runpy.run_path(str(PROJECT_ROOT / "tests/generate_camera_fk_fixtures.py"))
    expected = json.loads((PROJECT_ROOT / "runtime/tests/fixtures/camera_fk_v2.json").read_text())
    assert generator["build_fixture"]() == expected
