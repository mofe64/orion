"""Home installation is independent from Orion release switching."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import home_assistant_install as ha
import release_install
from pi_service_config import MANAGED_SERVICES

MANIFEST = Path(__file__).resolve().parents[1] / 'release-home-assistant.json'


class InstallerTests(unittest.TestCase):
    def run_install(self, root, run, **kwargs):
        return ha.install(Path(root), os.getuid(), os.getgid(), MANIFEST, run=run, **kwargs)
    def test_installs_docker_and_container_without_group_or_reboot_steps(self):
        def command(*args, **kwargs):
            if args[:3] == ('docker', 'container', 'inspect'): return subprocess.CompletedProcess(args, 1, '')
            return subprocess.CompletedProcess(args, 0, 'a' * 64 + '\n')
        run = Mock(side_effect=command)
        with tempfile.TemporaryDirectory() as root:
            self.run_install(root, run, which=lambda _: None)
            state = json.loads((Path(root) / '.local/share/orion/home-assistant-install.json').read_text())
            self.assertEqual(state['container_id'], 'a' * 64)
        commands = [call.args for call in run.call_args_list]
        self.assertIn(('env', 'DEBIAN_FRONTEND=noninteractive', 'apt-get', 'install', '-y', '--no-install-recommends', 'docker.io'), commands)
        self.assertIn(('systemctl', 'enable', '--now', 'docker.service'), commands)
        container = next(command for command in commands if command[:2] == ('docker', 'run'))
        for flag in ('--network=host', '--restart', 'unless-stopped', '--memory', '1g', '--cpu-shares', '256', '/run/dbus:/run/dbus:ro'):
            self.assertIn(flag, container)
        self.assertIn('@sha256:', container[-1]); self.assertIn(':2026.10.2@', container[-1])
        self.assertFalse(any('usermod' in command or 'reboot' in command for command in commands))
    def test_existing_container_is_unchanged_without_explicit_update(self):
        previous = {'Config': {'Image': 'older-version'}, 'State': {'Running': True}}
        run = Mock(return_value=subprocess.CompletedProcess([], 0, json.dumps([previous])))
        with tempfile.TemporaryDirectory() as root:
            result = self.run_install(root, run, which=lambda _: '/usr/bin/docker')
        self.assertFalse(result['changed'])
        self.assertEqual([call.args for call in run.call_args_list], [('docker', 'container', 'inspect', ha.NAME)])
    def test_update_pulls_before_stop_and_restores_previous_on_failure(self):
        def command(*args, **kwargs):
            if args[:3] == ('docker', 'container', 'inspect'):
                return subprocess.CompletedProcess(args, 0, json.dumps([{'Config': {'Image': 'old'}, 'State': {'Running': True}}]))
            if args[:2] == ('docker', 'run'): raise subprocess.CalledProcessError(1, args)
            return subprocess.CompletedProcess(args, 0, '')
        run = Mock(side_effect=command)
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaises(subprocess.CalledProcessError): self.run_install(root, run, update=True, which=lambda _: '/usr/bin/docker')
        commands = [call.args for call in run.call_args_list]
        self.assertLess(next(i for i, args in enumerate(commands) if args[:2] == ('docker', 'pull')), next(i for i, args in enumerate(commands) if args[:2] == ('docker', 'stop')))
        self.assertEqual(commands[-1], ('docker', 'start', ha.NAME))
    def test_stopped_or_interrupted_install_resumes_without_image_change(self):
        _, image = ha.image_pin(MANIFEST)
        previous = {'Id': 'b' * 64, 'Config': {'Image': image}, 'State': {'Running': False}}
        run = Mock(return_value=subprocess.CompletedProcess([], 0, json.dumps([previous])))
        with tempfile.TemporaryDirectory() as root:
            self.run_install(root, run, which=lambda _: '/usr/bin/docker')
            state = json.loads((Path(root) / '.local/share/orion/home-assistant-install.json').read_text())
            self.assertEqual(state['container_id'], 'b' * 64)
        self.assertEqual([call.args for call in run.call_args_list], [('docker', 'container', 'inspect', ha.NAME), ('docker', 'start', ha.NAME)])

    def test_matching_update_does_not_pull_restart_or_replace_container(self):
        _, image = ha.image_pin(MANIFEST)
        previous = {'Config': {'Image': image}, 'State': {'Running': True}}
        run = Mock(return_value=subprocess.CompletedProcess([], 0, json.dumps([previous])))
        with tempfile.TemporaryDirectory() as root:
            self.run_install(root, run, update=True, which=lambda _: '/usr/bin/docker')
        self.assertEqual(run.call_count, 1)

    def test_release_main_dispatches_only_home_step(self):
        with patch.object(sys, 'argv', ['release_install.py', '--root', '/tmp/orion-test', '--home-assistant', '--update']), patch.object(release_install, 'run') as run, patch.object(release_install, 'install') as install, patch.object(release_install, 'rollback') as rollback:
            release_install.main()
        install.assert_not_called(); rollback.assert_not_called()
        arguments = run.call_args.args
        self.assertEqual(arguments[:2], ('sudo', '/usr/bin/python3'))
        self.assertIn('--update', arguments)
        self.assertTrue(any(str(arg).endswith('home_assistant_install.py') for arg in arguments))
        self.assertNotIn(ha.NAME, MANAGED_SERVICES)
    def test_pins_are_required_and_update_requires_home_flag(self):
        with tempfile.TemporaryDirectory() as root:
            bad = Path(root) / 'manifest.json'; bad.write_text('{"image":"latest"}')
            with self.assertRaises(RuntimeError): ha.image_pin(bad)
        with patch.object(sys, 'argv', ['release_install.py', '--root', '/tmp/orion-test', '--update']):
            with self.assertRaises(SystemExit): release_install.main()
