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
    TorqueAnalysisError,
    analyze_dynamic_trajectory,
    analyze_named_poses,
    analyze_static_pose,
    load_motion_trajectory,
    load_named_pose,
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

    def test_supported_rest_is_not_reported_as_base_only_static_torque(self):
        with self.assertRaisesRegex(
            TorqueAnalysisError,
            "external mechanical support",
        ):
            analyze_static_pose(
                self.model,
                "rest",
                load_named_pose(DEFAULT_POSE_PATH, "rest"),
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

    def test_authored_motion_reports_dynamic_torque_and_speed(self):
        trajectory = load_motion_trajectory("look_at_left_expressive", "attentive")
        report = analyze_dynamic_trajectory(
            self.model,
            trajectory,
            start_pose_name="attentive",
            sample_period_seconds=0.05,
        )
        demands = {
            demand.joint_name: demand for demand in report.joint_demands
        }

        self.assertEqual(report.trajectory_name, "look_at_left_expressive")
        self.assertEqual(
            report.sample_count,
            1 + round(trajectory.total_duration / 0.05),
        )
        self.assertGreater(
            demands["base_yaw_joint"].peak_velocity_rad_s,
            0.0,
        )
        self.assertGreater(
            demands["elbow_pitch_joint"].rms_torque_nm,
            0.0,
        )

    def test_dynamic_analysis_rejects_non_positive_sample_period(self):
        trajectory = load_motion_trajectory("look_at_left_expressive", "attentive")

        with self.assertRaisesRegex(TorqueAnalysisError, "Sample period"):
            analyze_dynamic_trajectory(
                self.model,
                trajectory,
                start_pose_name="attentive",
                sample_period_seconds=0.0,
            )


if __name__ == "__main__":
    unittest.main()
