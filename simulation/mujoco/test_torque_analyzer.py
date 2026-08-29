"""Regression tests for Orion's static MuJoCo torque analysis."""

import sys
import unittest
from pathlib import Path

import mujoco


MUJOCO_DIRECTORY = Path(__file__).resolve().parent
sys.path.insert(0, str(MUJOCO_DIRECTORY))

from torque_analyzer import (  # noqa: E402
    DEFAULT_POSE_NAMES,
    DEFAULT_POSE_PATH,
    DEFAULT_SCENE_PATH,
    JOINT_NAMES,
    RATED_TORQUE_NM,
    TorqueAnalysisError,
    analyze_dynamic_trajectory,
    analyze_named_poses,
    analyze_static_pose,
    load_named_pose,
    load_pose_trajectory,
)


class StaticTorqueAnalysisTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.model = mujoco.MjModel.from_xml_path(str(DEFAULT_SCENE_PATH))

    def test_priority_poses_fit_the_native_model(self):
        reports = analyze_named_poses(
            DEFAULT_SCENE_PATH,
            DEFAULT_POSE_PATH,
            DEFAULT_POSE_NAMES,
        )

        self.assertEqual(
            tuple(report.pose_name for report in reports), DEFAULT_POSE_NAMES
        )

    def test_rest_report_transfers_support_to_the_physical_base(self):
        report = analyze_static_pose(
            self.model,
            "rest",
            load_named_pose(DEFAULT_POSE_PATH, "rest"),
        )
        demands = {
            demand.joint_name: demand for demand in report.joint_demands
        }

        self.assertAlmostEqual(report.base_support_force_n[2], 11.464916, places=5)
        self.assertAlmostEqual(
            demands["shoulder_pitch_joint"].torque_nm, -0.005367, places=5
        )
        self.assertAlmostEqual(
            demands["elbow_pitch_joint"].torque_nm, -0.728365, places=5
        )
        self.assertGreater(
            demands["elbow_pitch_joint"].absolute_torque_nm,
            RATED_TORQUE_NM,
        )

    def test_zero_pose_base_yaw_has_no_gravity_demand(self):
        report = analyze_static_pose(
            self.model,
            "zero_reference",
            load_named_pose(DEFAULT_POSE_PATH, "zero_reference"),
        )

        self.assertEqual(
            tuple(demand.joint_name for demand in report.joint_demands),
            JOINT_NAMES,
        )
        self.assertAlmostEqual(report.joint_demands[0].torque_nm, 0.0, places=10)

    def test_rest_to_attentive_reports_dynamic_torque_and_speed(self):
        validated = load_pose_trajectory("attentive", "rest", 6.0)
        report = analyze_dynamic_trajectory(
            self.model,
            validated,
            start_pose_name="rest",
            sample_period_seconds=0.05,
        )
        demands = {
            demand.joint_name: demand for demand in report.joint_demands
        }

        self.assertEqual(report.trajectory_name, "go_to_pose:attentive")
        self.assertEqual(report.sample_count, 121)
        self.assertGreater(
            demands["elbow_pitch_joint"].peak_absolute_torque_nm,
            RATED_TORQUE_NM,
        )
        self.assertGreater(
            demands["elbow_pitch_joint"].commissioning_velocity_fraction,
            1.0,
        )
        self.assertGreater(
            demands["elbow_pitch_joint"].rms_torque_nm,
            RATED_TORQUE_NM,
        )
        self.assertGreater(
            report.minimum_duration_for_velocity_setting_seconds,
            15.0,
        )

    def test_dynamic_analysis_rejects_non_positive_sample_period(self):
        validated = load_pose_trajectory("attentive", "rest", 6.0)

        with self.assertRaisesRegex(TorqueAnalysisError, "Sample period"):
            analyze_dynamic_trajectory(
                self.model,
                validated,
                start_pose_name="rest",
                sample_period_seconds=0.0,
            )


if __name__ == "__main__":
    unittest.main()
