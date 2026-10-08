"""Hardware profiles select services and preserve calibration separately."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
from pi_service_config import render_plan
from pi_catalog import built_in_yaml
from install_pi_voice_stack import System, activate, validate_catalog


class HardwareDeploymentTests(unittest.TestCase):
    def test_launcher_requires_explicit_hardware_before_attempting_network(self):
        result = subprocess.run(['bash', str(SCRIPTS / 'deploy_pi.sh'), '--skip-studio-check'],
                                env={k: v for k, v in os.environ.items() if k != 'ORION_PI_HARDWARE'}, capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn('Select the target hardware', result.stderr)

    def test_v2_replaces_effective_hardware_arguments_and_keeps_unrelated_tuning(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory); home = base / 'home'; home.mkdir()
            release = base / 'release'; release.mkdir()
            project = base / 'project'; project.mkdir()
            units = base / 'units'; units.mkdir()
            shutil.copytree(SCRIPTS / 'systemd', release / 'scripts/systemd')
            (release / 'release.json').write_text('{"revision":"test","hardware":"v2"}')
            override = units / 'oriond.service.d/99-local.conf'; override.parent.mkdir()
            override.write_text('[Service]\nExecStart=\nExecStart=/old/runtime/target/release/oriond --serve --hardware v1 --audio-card seeed2micvoicec --poses /old/v1.yaml --rest-after-seconds 1234 --character-on-start off\n')
            plan = render_plan(release, base / 'stack', project, home, 'pi', units)
            start = plan[override]
            for expected in ('--hardware v2', '--audio-card Array', '--rest-after-seconds 1234', '--character-on-start on', 'servo_calibration-v2.json', 'motion/config/v2/poses.yaml', 'motion/motions/v2'):
                self.assertIn(expected, start)
            self.assertNotIn('--hardware v1', start)
            # Earlier V2 releases wrote maintenance startup into the unit.
            self.assertNotIn('--character-on-start off', start)
            listener = plan[units / 'orion-listener.service']
            self.assertIn('--device plughw:CARD=Array,DEV=0', listener)
            self.assertIn('--hardware v2', listener)
            gateway = plan[units / 'orion-studio-gateway.service']
            self.assertIn('--hardware v2', gateway)
            self.assertIn('servo_calibration-v2.json', gateway)
            self.assertIn('Requires=orion-neopixel-pin.service', plan[units / 'oriond.service'])
            self.assertNotIn('ExecStartPre=', plan[units / 'oriond.service'])

    def test_v2_smoke_checks_lights_and_audio_before_settling_or_torque(self):
        with tempfile.TemporaryDirectory() as directory:
            release = Path(directory); (release / 'release.json').write_text('{"revision":"test","hardware":"v2"}')
            system = System(); system.wait_runtime_client = Mock(return_value=(['client'], {}))
            events = []
            system.settle_runtime = Mock(side_effect=lambda *_: events.append('rest'))
            system.run = Mock(side_effect=lambda *args, **kwargs: events.append(args) or Mock(stdout='{"mode":"configured"}'))
            system.stop_playback = Mock()
            system.smoke_runtime(release)
            self.assertEqual(events[0], ('client', '--run-scene', 'deployment_smoke', '--wait'))
            self.assertEqual(events[1], 'rest')

    def test_v2_release_validation_reads_incoming_repository_pose_file(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            release = base / 'release'; release.mkdir()
            (release / 'release.json').write_text('{"hardware":"v2"}')
            project = base / 'project'
            (project / 'motion').mkdir(parents=True)
            poses = project / 'motion/config/v2/poses.yaml'
            poses.parent.mkdir(parents=True)
            poses.write_text('old repository poses')
            motions = project / 'motion/motions/v2'; motions.mkdir(parents=True)
            (motions / 'return_home.yaml').write_text('motion')
            calibration = base / 'servo_calibration-v2.json'
            seen = []
            def run(*args, **kwargs):
                pose_file = Path(args[args.index('--pose-file') + 1])
                seen.append(pose_file.read_text())
                self.assertEqual(args[args.index('--calibration') + 1], calibration)
            system = Mock(); system.run.side_effect = run
            validate_catalog(release, project, calibration, system,
                             {poses: 'new repository poses'})
            self.assertEqual(seen, ['new repository poses'])
            self.assertEqual(poses.read_text(), 'old repository poses')

    def test_hardware_replacement_cannot_start_an_old_runtime_for_rest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); release = root / 'release'; release.mkdir()
            (release / 'release.json').write_text('{"revision":"test","hardware":"v2"}')
            (root / 'installation.json').write_text('{"version":2,"release":"old","hardware":"v1"}')
            system = Mock(); system.state.return_value = {'active': True, 'enabled': 'enabled'}
            with self.assertRaises(RuntimeError): activate(root, release, root, {}, system)
            system.stop.assert_not_called(); system.start.assert_not_called()

    def test_user_assets_are_preserved_and_builtin_v2_paths_are_managed(self):
        self.assertTrue(built_in_yaml('motion/motions/v2/nod.yaml'))
        self.assertFalse(built_in_yaml('motion/motions/v2/user/my_motion.yaml'))
        self.assertFalse(built_in_yaml('motion/user/poses/v2/home.yaml'))


if __name__ == '__main__':
    unittest.main()
