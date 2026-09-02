"""Native MuJoCo completion and stability regression tests."""

import sys
import tempfile
import unittest
from copy import deepcopy
from dataclasses import replace
from pathlib import Path

import mujoco
import yaml


MUJOCO_DIRECTORY = Path(__file__).resolve().parent
PROJECT_ROOT = MUJOCO_DIRECTORY.parents[1]
sys.path.insert(0, str(MUJOCO_DIRECTORY))

from motion_player import (  # noqa: E402
    CONFIG_DIRECTORY,
    execution_result_data,
    load_playback_data,
    load_stability_policy,
    run_playback_loop,
)
from mujoco_backend import (  # noqa: E402
    resolve_joint_mapping,
    set_joint_state,
)
from orion_motion.execution_types import ExecutionStatus  # noqa: E402
from orion_motion.compiled_trajectory import compile_trajectory  # noqa: E402
from orion_motion.motion_loader import load_yaml_file  # noqa: E402
from stability_monitor import (  # noqa: E402
    StabilityPolicyError,
    stability_policy_from_data,
)


class ClosedViewer:
    def is_running(self):
        return False


def make_simulation(trajectory, start_positions):
    model = mujoco.MjModel.from_xml_path(str(MUJOCO_DIRECTORY / "scene.xml"))
    data = mujoco.MjData(model)
    mapping = resolve_joint_mapping(model, trajectory.joint_names)
    set_joint_state(model, data, mapping, start_positions)
    return model, data, mapping


def aggressive_trajectory():
    poses = load_yaml_file(CONFIG_DIRECTORY / "poses.yaml")
    poses["poses"]["aggressive_test"] = {
        "description": "Intentionally aggressive simulator test pose.",
        "tags": ["simulation_test"],
        "positions": {
            "base_yaw_joint": -1.50,
            "shoulder_pitch_joint": 0.75,
            "elbow_pitch_joint": -0.95,
            "head_roll_joint": 1.40,
            "head_pitch_joint": 0.65,
        },
    }
    motion = {
        "format_version": 2,
        "motion": {
            "name": "aggressive_test",
            "description": "Negative stability test only.",
            "space": "absolute",
            "style": "quick_reaction",
            "keyframes": [
                {
                    "pose": "aggressive_test",
                    "duration": 0.1,
                    "arrival": "settle",
                    "hold": 0.5,
                }
            ],
        },
    }
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        pose_path = root / "poses.yaml"
        motions = root / "motions"
        motions.mkdir()
        pose_path.write_text(yaml.safe_dump(poses, sort_keys=False), encoding="utf-8")
        (motions / "aggressive_test.yaml").write_text(
            yaml.safe_dump(motion, sort_keys=False), encoding="utf-8"
        )
        trajectory = compile_trajectory(
            "aggressive_test",
            "attentive",
            pose_file=pose_path,
            motions_directory=motions,
        )
    return trajectory, trajectory.points[0].positions


class NativeExecutionTests(unittest.TestCase):
    def test_slow_motion_records_feedback_and_settles(self):
        _, trajectory, start = load_playback_data("look_at_left", "attentive")
        model, data, mapping = make_simulation(trajectory, start)

        result = run_playback_loop(
            model,
            data,
            mapping,
            trajectory,
            lead_in=0.2,
            viewer=None,
        )

        self.assertEqual(result.status, ExecutionStatus.SUCCEEDED)
        self.assertTrue(result.feedback)
        self.assertIsNotNone(result.metrics)
        self.assertGreaterEqual(result.metrics.settling_time, 0.25)
        self.assertLess(max(result.metrics.final_position_errors), 0.05)
        self.assertLess(result.metrics.maximum_base_translation, 0.01)
        self.assertEqual(execution_result_data(result)["status"], "succeeded")

    def test_time_elapsed_without_measured_settling_is_failure(self):
        _, trajectory, start = load_playback_data("look_at_left", "attentive")
        model, data, mapping = make_simulation(trajectory, start)
        strict_policy = replace(
            load_stability_policy(),
            position_tolerance=1e-8,
            velocity_tolerance=1e-8,
            settle_timeout=0.05,
        )

        result = run_playback_loop(
            model,
            data,
            mapping,
            trajectory,
            lead_in=0.2,
            viewer=None,
            policy=strict_policy,
        )

        self.assertEqual(result.status, ExecutionStatus.SETTLING_FAILED)
        self.assertFalse(result.succeeded)

    def test_aggressive_motion_is_rejected_by_stability_measurement(self):
        trajectory, start = aggressive_trajectory()
        model, data, mapping = make_simulation(trajectory, start)

        result = run_playback_loop(
            model,
            data,
            mapping,
            trajectory,
            lead_in=1.0,
            viewer=None,
        )

        self.assertEqual(result.status, ExecutionStatus.UNSAFE_STABILITY)
        self.assertTrue(
            result.metrics.maximum_base_translation > 0.01
            or result.metrics.maximum_base_tilt > 0.087
        )

    def test_closing_viewer_is_cancellation_not_success(self):
        _, trajectory, start = load_playback_data("look_at_left", "attentive")
        model, data, mapping = make_simulation(trajectory, start)

        result = run_playback_loop(
            model,
            data,
            mapping,
            trajectory,
            lead_in=1.0,
            viewer=ClosedViewer(),
        )

        self.assertEqual(result.status, ExecutionStatus.CANCELLED)
        self.assertFalse(result.succeeded)


class StabilityPolicyTests(unittest.TestCase):
    def test_policy_rejects_non_positive_threshold(self):
        data = deepcopy(load_yaml_file(CONFIG_DIRECTORY / "stability_limits.yaml"))
        data["base"]["maximum_tilt"] = 0.0

        with self.assertRaisesRegex(StabilityPolicyError, "maximum_tilt"):
            stability_policy_from_data(data)


if __name__ == "__main__":
    unittest.main()
