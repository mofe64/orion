"""Check CAD integration against analytic boxes and both simulator formats."""
import json
from pathlib import Path
import sys
import unittest
import xml.etree.ElementTree as ET

import mujoco
import numpy as np

PROJECT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(PROJECT / 'scripts'))
from estimate_v2_inertias import combine, solid_properties


class InertiaTests(unittest.TestCase):
    def box(self):
        points = np.array([[0, 0, 0], [2, 0, 0], [2, 3, 0], [0, 3, 0],
                           [0, 0, 4], [2, 0, 4], [2, 3, 4], [0, 3, 4]], dtype=float)
        faces = [[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7],
                 [0, 1, 5], [0, 5, 4], [1, 2, 6], [1, 6, 5],
                 [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]]
        return points[np.array(faces)]

    def test_mesh_volume_centre_and_tensor_match_an_analytic_box(self):
        for triangles in (self.box(), self.box()[:, ::-1]):
            volume, centre, inertia = solid_properties(triangles + [8, -10, 6])
            self.assertAlmostEqual(volume, 24)
            np.testing.assert_allclose(centre, [9, -8.5, 8])
            np.testing.assert_allclose(inertia, np.diag([50, 40, 26]), atol=1e-10)
        with self.assertRaises(ValueError): solid_properties(self.box()[:-1])

    def test_link_aggregation_uses_the_parallel_axis_theorem(self):
        parts = [dict(mass_kg=2, centre_m=[x, 0, 0], inertia_kg_m2=np.eye(3).tolist()) for x in (-1, 1)]
        combined = combine(parts)
        self.assertEqual(combined['mass_kg'], 4)
        np.testing.assert_allclose(combined['centre_m'], [0, 0, 0])
        np.testing.assert_allclose(combined['inertia_kg_m2'], np.diag([2, 6, 6]))

    def test_packaged_masses_and_inertias_match_urdf_and_mujoco(self):
        root = PROJECT / 'simulation/mujoco/v2'
        report = json.loads((root / 'mass-properties.json').read_text())
        self.assertEqual(report['scope'], 'unmeasured_mass_estimate')
        self.assertFalse(report['dynamic_validation'])
        model = mujoco.MjModel.from_xml_path(str(root / 'scene.xml'))
        urdf = ET.parse(PROJECT / 'description/urdf/orion-v2.urdf').getroot()
        for name, expected in report['links'].items():
            body_id = mujoco.mj_name2id(model, mujoco.mjtObj.mjOBJ_BODY, name)
            self.assertAlmostEqual(model.body_mass[body_id], expected['mass_kg'])
            np.testing.assert_allclose(model.body_ipos[body_id], expected['centre_m'])
            rotation = np.empty(9); mujoco.mju_quat2Mat(rotation, model.body_iquat[body_id])
            rotation = rotation.reshape(3, 3)
            actual = rotation @ np.diag(model.body_inertia[body_id]) @ rotation.T
            # MuJoCo diagonalizes fullinertia with an iterative eigensolver.
            np.testing.assert_allclose(actual, expected['inertia_kg_m2'], atol=1e-9)
            inertial = urdf.find(f"link[@name='{name}']/inertial")
            self.assertAlmostEqual(float(inertial.find('mass').get('value')), expected['mass_kg'])
            matrix = np.array(expected['inertia_kg_m2'])
            for attr, i, j in [('ixx', 0, 0), ('iyy', 1, 1), ('izz', 2, 2), ('ixy', 0, 1), ('ixz', 0, 2), ('iyz', 1, 2)]:
                self.assertAlmostEqual(float(inertial.find('inertia').get(attr)), matrix[i, j])


if __name__ == '__main__':
    unittest.main()
