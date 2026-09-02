"""Regression tests for the calibrated MuJoCo pose editor."""

import copy
import json
import math
import sys
import tempfile
import unittest
from pathlib import Path


MUJOCO_DIRECTORY = Path(__file__).resolve().parent
PROJECT_ROOT = MUJOCO_DIRECTORY.parents[1]
sys.path.insert(0, str(MUJOCO_DIRECTORY))

from pose_editor import (  # noqa: E402
    CANONICAL_JOINTS,
    load_calibrated_limits,
    load_editor_configuration,
    replace_pose_positions,
    save_pose,
)


SIMULATION_CALIBRATION = (
    MUJOCO_DIRECTORY / "config" / "servo_calibration.json"
)
POSE_LIBRARY = (
    PROJECT_ROOT / "motion" / "config" / "poses.yaml"
)


class CalibratedLimitsTests(unittest.TestCase):
    def test_converts_safe_encoder_deltas_to_radians(self):
        limits = load_calibrated_limits(SIMULATION_CALIBRATION)

        self.assertEqual(tuple(limits), CANONICAL_JOINTS)
        self.assertAlmostEqual(
            limits["base_yaw_joint"][0], -1004 * 2 * math.pi / 4096
        )
        self.assertAlmostEqual(
            limits["shoulder_pitch_joint"][1], 525 * 2 * math.pi / 4096
        )

    def test_reversed_encoder_direction_reverses_and_sorts_angle_bounds(self):
        calibration = json.loads(SIMULATION_CALIBRATION.read_text(encoding="utf-8"))
        calibration["joints"]["shoulder_pitch_joint"]["encoder_direction"] = -1
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "calibration.json"
            path.write_text(json.dumps(calibration), encoding="utf-8")
            limits = load_calibrated_limits(path)

        self.assertAlmostEqual(
            limits["shoulder_pitch_joint"][0], -525 * 2 * math.pi / 4096
        )
        self.assertAlmostEqual(
            limits["shoulder_pitch_joint"][1], 933 * 2 * math.pi / 4096
        )


class PoseLibraryTests(unittest.TestCase):
    def test_loads_every_pose_in_yaml_order(self):
        configuration = load_editor_configuration(SIMULATION_CALIBRATION, POSE_LIBRARY)

        self.assertEqual(
            configuration.pose_names[0:3], ("zero_reference", "rest", "home")
        )
        self.assertEqual(configuration.pose_names[-1], "look_right_overshoot")
        self.assertEqual(len(configuration.pose_names), 15)

    def test_rejects_pose_outside_physical_calibration(self):
        source = POSE_LIBRARY.read_text(encoding="utf-8")
        invalid = source.replace(
            "      shoulder_pitch_joint: 0.0",
            "      shoulder_pitch_joint: 0.90000000",
            1,
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "poses.yaml"
            path.write_text(invalid, encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "outside its calibrated range"):
                load_editor_configuration(SIMULATION_CALIBRATION, path)

    def test_replaces_only_selected_pose_values(self):
        source = POSE_LIBRARY.read_text(encoding="utf-8")
        configuration = load_editor_configuration(SIMULATION_CALIBRATION, POSE_LIBRARY)
        targets = configuration.poses["home"].copy()
        targets["base_yaw_joint"] = -0.25

        updated = replace_pose_positions(source, "home", targets)

        self.assertIn("      base_yaw_joint: -0.25000000", updated)
        before_home, after_home = source.split("  home:", 1)
        updated_before_home, updated_after_home = updated.split("  home:", 1)
        self.assertEqual(updated_before_home, before_home)
        self.assertEqual(
            updated_after_home.split("  attentive:", 1)[1],
            after_home.split("  attentive:", 1)[1],
        )
        self.assertIn(
            "description: Compact powered resting pose and default character anchor.",
            updated,
        )

    def test_save_is_reloadable_and_preserves_other_poses(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "poses.yaml"
            path.write_text(POSE_LIBRARY.read_text(encoding="utf-8"), encoding="utf-8")
            configuration = load_editor_configuration(SIMULATION_CALIBRATION, path)
            rest_before = copy.deepcopy(configuration.poses["rest"])
            targets = configuration.poses["home"].copy()
            targets["head_pitch_joint"] = 0.15

            save_pose(configuration, "home", targets)
            reloaded = load_editor_configuration(SIMULATION_CALIBRATION, path)

        self.assertEqual(reloaded.poses["rest"], rest_before)
        self.assertEqual(reloaded.poses["home"]["head_pitch_joint"], 0.15)


if __name__ == "__main__":
    unittest.main()
