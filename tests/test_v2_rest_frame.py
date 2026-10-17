"""Immutable pre-migration FK evidence, independent of the rest-frame formulas."""
import json
import math
from pathlib import Path
import xml.etree.ElementTree as ET
import mujoco
import numpy as np
import yaml

ROOT = Path(__file__).resolve().parents[1]


def test_every_pose_preserves_baseline_fk_before_separate_photo_alignment():
    reference = json.loads((ROOT / 'simulation/mujoco/config/model_reference-v2.json').read_text())
    baseline = json.loads((ROOT / 'tests/fixtures/v2-rest-migration.json').read_text())
    poses = yaml.safe_load((ROOT / 'motion/config/v2/poses.yaml').read_text())['poses']
    tree = ET.parse(ROOT / 'simulation/mujoco/v2/robot.xml')
    # Photo correction deliberately changes geometry; reverse only that recorded
    # correction to evaluate the exact intermediate migration, as requested.
    for joint in tree.findall('.//joint'):
        if joint.get('name') in reference['photo_alignment']['migration_refs_before_photo_alignment']:
            joint.set('ref', str(reference['photo_alignment']['migration_refs_before_photo_alignment'][joint.get('name')]))
    for mesh in tree.findall('./asset/mesh'):
        mesh.set('file', str(ROOT / 'simulation/mujoco/v2' / mesh.get('file')))
    model = mujoco.MjModel.from_xml_string(ET.tostring(tree.getroot(), encoding='unicode'))
    data = mujoco.MjData(model)
    assert set(poses) == set(baseline['cases'])
    for name, expected in baseline['cases'].items():
        for joint, old_angle in expected['old_angles'].items():
            angle = poses[name]['positions'][joint]
            assert abs(angle - (old_angle - reference['rest_frame_migration']['rest_shift_radians'][joint])) < 1e-12
            data.qpos[model.joint(joint).qposadr[0]] = angle
        mujoco.mj_forward(model, data)
        errors = np.linalg.norm(data.xpos - expected['body_positions_metres'], axis=1)
        assert errors.max() < .0001, (name, errors.max())
        np.testing.assert_allclose(data.xmat, expected['body_rotation_matrices'], atol=1e-12)


def test_rest_is_zero_and_photo_aligned_pitch_axes_match_measurements():
    poses = yaml.safe_load((ROOT / 'motion/config/v2/poses.yaml').read_text())['poses']
    assert all(value == 0 for value in poses['rest']['positions'].values())
    reference = json.loads((ROOT / 'simulation/mujoco/config/model_reference-v2.json').read_text())
    measured = reference['photo_alignment']['measured_degrees']
    model = mujoco.MjModel.from_xml_path(str(ROOT / 'simulation/mujoco/v2/scene.xml'))
    data = mujoco.MjData(model); data.qpos[:] = 0; mujoco.mj_forward(model, data)
    shoulder, elbow, wrist = (data.body(n).xpos for n in ('v21_shoulder_pitch_link', 'v21_elbow_pitch_link', 'v21_wrist_pitch_link'))
    # Facing -Y; undo the tiny recorded yaw reference for the side projection.
    for vector, key in [(elbow - shoulder, 'upper_arm_lean_from_vertical'), (wrist - elbow, 'forearm_lean_from_vertical')]:
        magnitude = math.hypot(vector[0], vector[1])
        angle = math.degrees(math.atan2(-math.copysign(magnitude, vector[1]), vector[2]))
        assert abs(angle - measured[key]) < .1
    axis = data.site('light_frame').xmat.reshape(3, 3)[:, 2]
    depression = math.degrees(math.atan2(-axis[2], math.hypot(axis[0], axis[1])))
    assert abs(depression - measured['head_depression_from_horizontal']) < .1
