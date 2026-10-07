"""V2 uses the existing calibration and stable-rest capture routines."""
import io
import json
import shutil
import tempfile
from contextlib import redirect_stdout
from pathlib import Path
import unittest
from unittest.mock import patch

import yaml
from orion_servo_setup.calibration import initialize_captures, update_captures, build_calibration_document, CalibrationError, load_hardware_calibration, write_calibration_file
from orion_servo_setup.provisioning import assignments_for_hardware
from orion_servo_setup.calibrate_cli import main
from orion_servo_setup.rest_cli import main as capture_rest
from orion_servo_setup.rest_cli import ORION_ROOT
from test_calibrate_cli import FakeCalibrationBus
from test_rest_cli import FakeRestBus, calibration_document


class V2CalibrationTests(unittest.TestCase):
    def captures(self):
        assignments = assignments_for_hardware('v2')
        captures = initialize_captures({a.joint_name: 2048 for a in assignments}, assignments)
        for raw in (1400, 2700):
            captures = update_captures(captures, {a.joint_name: raw for a in assignments})
        return captures

    def test_direction_values_and_physical_names_survive_roundtrip(self):
        assignments = assignments_for_hardware('v2')
        self.assertEqual(assignments[3].joint_ref_name, 'neck_swivel')
        self.assertEqual(assignments[4].joint_ref_name, 'wrist_pitch')
        directions = {a.joint_name: -1 if a.servo_id == 5 else 1 for a in assignments}
        document = build_calibration_document(self.captures(), port='fake', hardware='v2', directions=directions)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'v2.json'; write_calibration_file(document, path)
            with self.assertRaises(CalibrationError): load_hardware_calibration(path, 'v1')
            self.assertEqual(load_hardware_calibration(path, 'v2')['head_pitch_joint'].encoder_direction, -1)
        self.assertEqual(document['joints']['head_pitch_joint']['joint_ref_name'], 'wrist_pitch')
        self.assertNotIn('directions_verified', document)

    def test_v2_uses_existing_calibration_session_without_approval_flags(self):
        bus = FakeCalibrationBus()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'v2.json'
            with patch('orion_servo_setup.calibrate_cli.create_lerobot_bus', return_value=bus), \
                 patch('orion_servo_setup.calibrate_cli._record_until_enter', return_value=self.captures()), \
                 patch('builtins.input', side_effect=['', '']), redirect_stdout(io.StringIO()):
                self.assertEqual(main(['--hardware', 'v2', '--port', 'fake', '--output', str(path)]), 0)
            document = json.loads(path.read_text())
            self.assertEqual(document['hardware'], 'v2')
            self.assertEqual(len(load_hardware_calibration(path, 'v2')), 5)
            self.assertNotIn('directions_verified', document)
            self.assertEqual(bus.disable_calls, [(None, 2)])
            self.assertEqual(bus.disconnect_calls, [True])

    def test_v2_rest_capture_updates_repository_poses_and_preserves_home(self):
        with tempfile.TemporaryDirectory() as directory:
            calibration = Path(directory) / 'v2.json'
            document = calibration_document(); document['hardware'] = 'v2'
            calibration.write_text(json.dumps(document))
            poses = Path(directory) / 'motion/config/v2/poses.yaml'
            poses.parent.mkdir(parents=True)
            shutil.copyfile(ORION_ROOT / 'motion/config/v2/poses.yaml', poses)
            original_home = yaml.safe_load(poses.read_text())['poses']['home']
            bus = FakeRestBus()
            with patch('builtins.input', side_effect=['', '', 'y']), \
                 patch('orion_servo_setup.rest_cli.ORION_ROOT', Path(directory)), \
                 patch('orion_servo_setup.rest_cli.create_lerobot_bus', return_value=bus), \
                 patch('orion_servo_setup.rest_cli.read_preflight'), \
                 patch('orion_servo_setup.rest_cli.time.sleep'), \
                 patch('orion_servo_setup.rest_cli.time.monotonic', side_effect=[0.0, 0.0, 5.0]), \
                 redirect_stdout(io.StringIO()):
                self.assertEqual(capture_rest(['--hardware', 'v2', '--port', 'fake',
                    '--calibration', str(calibration)]), 0)
            saved = yaml.load(poses.read_text(), Loader=yaml.BaseLoader)
            self.assertEqual(saved['poses']['rest']['default_lighting'], 'off')
            self.assertIn('mechanical_rest', saved['poses']['rest']['tags'])
            self.assertEqual({float(value) for value in saved['poses']['rest']['positions'].values()}, {0.0})
            self.assertEqual(yaml.safe_load(poses.read_text())['poses']['home'], original_home)
            self.assertGreaterEqual(bus.disable_calls, 1)
            self.assertFalse(bus.is_connected)
