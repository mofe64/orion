"""Portable resources and identical URDF/MJCF kinematics for the V2.1 lamp."""
import hashlib
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET

import mujoco
import yaml
import numpy as np

PROJECT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(PROJECT / 'runtime'))
from mujoco_bridge import Bridge, JOINT_NAMES


class V2ModelTests(unittest.TestCase):
    def test_portable_model_and_mesh_provenance_survive_relocation(self):
        model_root = PROJECT / 'simulation/mujoco/v2'
        provenance = json.loads((model_root / 'provenance.json').read_text())
        for mesh in provenance['meshes']:
            self.assertFalse(Path(mesh['file']).is_absolute())
            self.assertEqual(hashlib.sha256((model_root / mesh['file']).read_bytes()).hexdigest(), mesh['sha256'])
        with tempfile.TemporaryDirectory() as directory:
            copy = Path(directory) / 'v2'; shutil.copytree(model_root, copy)
            model = mujoco.MjModel.from_xml_path(str(copy / 'scene.xml'))
            self.assertEqual(model.nq, 5); self.assertEqual(model.nu, 5)
            self.assertTrue(np.all(model.geom_contype == 0))

    def test_urdf_and_mujoco_forward_kinematics_match_in_metres_and_radians(self):
        robot = ET.parse(PROJECT / 'description/urdf/orion-v2.urdf').getroot()
        poses = yaml.safe_load((PROJECT / 'motion/config/v2/poses.yaml').read_text())['poses']
        bridge = Bridge(PROJECT / 'simulation/mujoco/v2/scene.xml', poses['home']['positions'], 'v2')
        for name in ('home', 'look_left', 'look_right', 'zero_reference'):
            angles = poses[name]['positions']
            bridge.handle({'command': 'write', 'positions': angles})
            transforms = {'v21_base': np.eye(4)}
            for joint in robot.findall('joint'):
                parent, child = joint.find('parent').get('link'), joint.find('child').get('link')
                origin = np.fromstring(joint.find('origin').get('xyz'), sep=' ')
                relative = np.eye(4); relative[:3, 3] = origin
                if joint.get('type') == 'revolute':
                    axis = np.fromstring(joint.find('axis').get('xyz'), sep=' ')
                    a = angles[joint.get('name')]; x, y, z = axis
                    cross = np.array([[0, -z, y], [z, 0, -x], [-y, x, 0]])
                    relative[:3, :3] = np.eye(3) + np.sin(a) * cross + (1 - np.cos(a)) * cross @ cross
                transforms[child] = transforms[parent] @ relative
            expected_face = transforms['v21_neck_swivel_link'] @ np.array([0, -.085, .020, 1])
            site = mujoco.mj_name2id(bridge.model, mujoco.mjtObj.mjOBJ_SITE, 'v21_face')
            np.testing.assert_allclose(bridge.data.site_xpos[site], expected_face[:3], atol=1e-10)

    def test_look_left_turns_toward_the_lamps_own_left_like_the_hardware(self):
        # The lamp faces -Y, so its own left is +X. On the fitted lamp,
        # look_left turns to its own left (checked 2026-10-08).
        poses = yaml.safe_load((PROJECT / 'motion/config/v2/poses.yaml').read_text())['poses']
        model = mujoco.MjModel.from_xml_path(str(PROJECT / 'simulation/mujoco/v2/scene.xml'))
        data = mujoco.MjData(model)
        def face_x(pose):
            data.qpos[:] = 0
            for joint, value in poses[pose]['positions'].items():
                data.qpos[model.joint(joint).qposadr[0]] = value
            mujoco.mj_forward(model, data)
            return data.site('v21_face').xpos[0]
        self.assertGreater(face_x('look_left'), 0.05)
        self.assertLess(face_x('look_right'), -0.05)

    def test_bridge_rejects_wrong_hardware_and_never_claims_dynamic_safety(self):
        poses = yaml.safe_load((PROJECT / 'motion/config/v2/poses.yaml').read_text())['poses']['home']['positions']
        scene = PROJECT / 'simulation/mujoco/v2/scene.xml'
        with self.assertRaises(ValueError): Bridge(scene, poses, 'v1')
        bridge = Bridge(scene, poses, 'v2')
        metrics = bridge.handle({'command': 'activate'})['metrics']
        self.assertEqual(metrics['validation_scope'], 'kinematic_preview')
        self.assertFalse(metrics['safe']); self.assertTrue(metrics['unsafe_reasons'])
