"""Simulator-level regression tests for Orion's MuJoCo mapping boundary."""

import sys
import unittest
from pathlib import Path

import mujoco
import numpy as np


MUJOCO_DIRECTORY = Path(__file__).resolve().parent
sys.path.insert(0, str(MUJOCO_DIRECTORY))

from mujoco_backend import (  # noqa: E402
    DEFAULT_BASE_BODY_NAME,
    read_joint_positions,
    read_joint_velocities,
    resolve_joint_mapping,
    set_joint_state,
)
from motion_player import load_playback_data, run_playback_loop  # noqa: E402
from pose_tuner import load_pose_configuration  # noqa: E402
from orion_motion.compiled_trajectory import sample_trajectory  # noqa: E402


JOINT_NAMES = (
    "base_yaw_joint",
    "shoulder_pitch_joint",
    "elbow_pitch_joint",
    "head_roll_joint",
    "head_pitch_joint",
)


def body_transform(model, data, body_name):
    body_id = mujoco.mj_name2id(model, mujoco.mjtObj.mjOBJ_BODY, body_name)
    return (
        data.xpos[body_id].copy(),
        data.xmat[body_id].reshape(3, 3).copy(),
    )


def rotation_change_radians(reference, current):
    relative = reference.T @ current
    cosine = np.clip((np.trace(relative) - 1.0) / 2.0, -1.0, 1.0)
    return float(np.arccos(cosine))


class SetJointStateTests(unittest.TestCase):
    def setUp(self):
        self.model = mujoco.MjModel.from_xml_path(
            str(MUJOCO_DIRECTORY / "scene.xml")
        )
        self.data = mujoco.MjData(self.model)
        self.mapping = resolve_joint_mapping(self.model, JOINT_NAMES)

    def test_nonzero_yaw_preserves_base_and_rotates_upper_assembly(self):
        zero_yaw = (0.0, -0.1, -0.28, -0.65, -0.22)
        forward_yaw = (-0.3, -0.1, -0.28, -0.65, -0.22)

        set_joint_state(self.model, self.data, self.mapping, zero_yaw)
        base_position_before, base_rotation_before = body_transform(
            self.model, self.data, DEFAULT_BASE_BODY_NAME
        )
        _, upper_rotation_before = body_transform(
            self.model, self.data, "lamparm__base_elbow"
        )

        set_joint_state(self.model, self.data, self.mapping, forward_yaw)
        base_position_after, base_rotation_after = body_transform(
            self.model, self.data, DEFAULT_BASE_BODY_NAME
        )
        _, upper_rotation_after = body_transform(
            self.model, self.data, "lamparm__base_elbow"
        )

        np.testing.assert_allclose(
            base_position_after, base_position_before, atol=1e-10
        )
        np.testing.assert_allclose(
            base_rotation_after, base_rotation_before, atol=1e-10
        )
        self.assertAlmostEqual(
            rotation_change_radians(upper_rotation_before, upper_rotation_after),
            0.3,
            places=6,
        )
        self.assertAlmostEqual(
            read_joint_positions(self.data, self.mapping)[0], -0.3, places=10
        )
        self.assertEqual(read_joint_velocities(self.data, self.mapping), (0.0,) * 5)

    def test_zero_keyframe_preserves_the_default_base_transform(self):
        mujoco.mj_forward(self.model, self.data)
        base_position_before, base_rotation_before = body_transform(
            self.model, self.data, DEFAULT_BASE_BODY_NAME
        )
        keyframe_id = mujoco.mj_name2id(
            self.model, mujoco.mjtObj.mjOBJ_KEY, "zero_reference"
        )

        mujoco.mj_resetDataKeyframe(self.model, self.data, keyframe_id)
        mujoco.mj_forward(self.model, self.data)

        base_position_after, base_rotation_after = body_transform(
            self.model, self.data, DEFAULT_BASE_BODY_NAME
        )
        np.testing.assert_allclose(
            base_position_after, base_position_before, atol=1e-12
        )
        np.testing.assert_allclose(
            base_rotation_after, base_rotation_before, atol=1e-12
        )
        self.assertEqual(
            read_joint_positions(self.data, self.mapping), (0.0,) * len(JOINT_NAMES)
        )


class PoseTunerConfigurationTests(unittest.TestCase):
    def test_pose_tuner_reads_current_operational_limit_schema(self):
        configuration = load_pose_configuration("rest")

        self.assertEqual(configuration.joint_order, JOINT_NAMES)
        lower, upper = configuration.limits["shoulder_pitch_joint"]
        self.assertAlmostEqual(lower, -1.431204075097303)
        self.assertAlmostEqual(upper, 0.805339913639962)
        self.assertAlmostEqual(
            configuration.initial_targets["elbow_pitch_joint"],
            1.03697101,
        )


class SharedTrajectoryPlaybackTests(unittest.TestCase):
    def test_mujoco_loads_the_shared_generated_trajectory(self):
        _, trajectory, start_positions = load_playback_data(
            "look_at_left", "attentive"
        )

        self.assertEqual(trajectory.points[0].positions, start_positions)
        self.assertEqual(
            trajectory.joint_names,
            JOINT_NAMES,
        )

        midpoint, sample_index = sample_trajectory(trajectory, 0.75)
        self.assertLess(sample_index, len(trajectory.points) - 1)
        self.assertNotEqual(midpoint.positions, trajectory.points[-1].positions)

    def test_mujoco_execution_rejects_non_rust_trajectory(self):
        _, trajectory, _ = load_playback_data("look_at_left", "attentive")

        with self.assertRaisesRegex(TypeError, "Rust CompiledTrajectory"):
            run_playback_loop(
                None,
                None,
                None,
                trajectory.points,
                lead_in=0.0,
                viewer=None,
            )


if __name__ == "__main__":
    unittest.main()
