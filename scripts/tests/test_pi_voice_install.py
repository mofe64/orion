import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('retire', SCRIPTS / 'retire_pi_voice.py')
retire = importlib.util.module_from_spec(spec)
spec.loader.exec_module(retire)


class RetirementTests(unittest.TestCase):
    def test_only_checkout_workers_and_their_children_are_selected(self):
        root = Path('/home/pi/orion')
        processes = {
            1: (0, ['/usr/bin/python', 'unrelated.py'], '1'),
            2: (1, [str(root / 'voice/.venv/bin/orion-voice'), 'listen-worker'], '2'),
            3: (2, ['arecord', '-c', '1'], '3'),
            4: (1, ['/elsewhere/voice/.venv/bin/orion-voice', 'listen-worker'], '4'),
            5: (1, [str(root / 'voice/.venv/bin/python'), '-m', 'orion_voice', 'tts-worker'], '5'),
        }
        self.assertEqual(retire.legacy_processes(processes, root), {2, 3, 5})
        self.assertTrue(retire.is_legacy_command(['path=/home/pi/orion/voice/.venv/bin/orion-voice', 'argv[]=/home/pi/orion/voice/.venv/bin/orion-voice', 'wake-worker'], root))

    def test_only_matching_system_and_user_units_are_disabled(self):
        root = Path('/home/pi/orion')
        calls = []
        def run(arguments, **kwargs):
            calls.append(arguments)
            output = ''
            if 'list-unit-files' in arguments:
                output = 'orion-voice.service enabled\noriond.service enabled\nother.service enabled\n'
            elif 'show' in arguments:
                unit = arguments[arguments.index('show') + 1]
                if unit == 'orion-voice.service':
                    output = '{ path=/home/pi/orion/voice/.venv/bin/orion-voice ; argv[]=/home/pi/orion/voice/.venv/bin/orion-voice listen-worker ; }'
                elif unit == 'other.service':
                    output = '/another/voice/.venv/bin/orion-voice listen-worker'
                else:
                    output = '/home/pi/orion/runtime/target/release/oriond --serve'
            return subprocess.CompletedProcess(arguments, 0, output, '')
        with patch.object(retire.subprocess, 'run', run):
            retire.retire_services(root)
        disabled = [args for args in calls if 'disable' in args]
        self.assertEqual(len(disabled), 2)
        self.assertTrue(all(args[-1] == 'orion-voice.service' for args in disabled))
        self.assertTrue(any('--user' in args for args in disabled))

    def test_templates_are_skipped_but_installed_and_loaded_instances_are_retired(self):
        root = Path('/home/pi/orion')
        calls = []
        def run(arguments, **kwargs):
            calls.append(arguments)
            if 'list-unit-files' in arguments:
                output = ('autovt@.service alias -\nlegacy@.service disabled enabled\n'
                          'legacy@boot.service enabled enabled\n'
                          'old-voice.service disabled enabled\n\n')
            elif 'list-units' in arguments:
                self.assertIn('--all', arguments)
                self.assertIn('--plain', arguments)
                self.assertIn('--full', arguments)
                output = ('legacy@live.service loaded active running Voice\n'
                          'legacy@boot.service loaded active running Voice\n'
                          'getty@tty1.service loaded active running Login\n')
            elif 'show' in arguments:
                unit = arguments[arguments.index('show') + 1]
                # Reproduce the Pi failure if the implementation queries a template.
                if unit.endswith('@.service'):
                    if kwargs.get('check'):
                        raise subprocess.CalledProcessError(1, arguments, stderr='Invalid argument')
                    return subprocess.CompletedProcess(arguments, 1, '', 'Invalid argument')
                if unit == 'getty@tty1.service':
                    output = '{ path=/sbin/agetty ; argv[]=/sbin/agetty tty1 ; }'
                else:
                    output = '{ path=/home/pi/orion/voice/.venv/bin/orion-voice ; argv[]=/home/pi/orion/voice/.venv/bin/orion-voice wake-worker ; }'
            else:
                output = ''
            return subprocess.CompletedProcess(arguments, 0, output, '')
        with patch.object(retire.subprocess, 'run', run):
            retire.retire_services(root)
        disabled = [args for args in calls if 'disable' in args]
        for user_scope in (False, True):
            units = [args[-1] for args in disabled if ('--user' in args) == user_scope]
            self.assertCountEqual(units, ['legacy@boot.service', 'legacy@live.service', 'old-voice.service'])
        self.assertFalse(any(args[args.index('show') + 1].endswith('@.service')
                             for args in calls if 'show' in args))

    def test_inspection_errors_are_actionable_and_do_not_allow_retirement(self):
        for failed_command in ('list-unit-files', 'list-units', 'show'):
            with self.subTest(failed_command=failed_command):
                calls = []
                def run(arguments, **kwargs):
                    calls.append(arguments)
                    if failed_command in arguments:
                        return subprocess.CompletedProcess(arguments, 1, '', 'Access denied')
                    return subprocess.CompletedProcess(arguments, 0, 'old-voice.service enabled\n', '')
                with patch.object(retire.subprocess, 'run', run):
                    with self.assertRaisesRegex(RuntimeError, r'system services.*Access denied'):
                        retire.retire_services(Path('/home/pi/orion'))
                self.assertFalse(any('disable' in args for args in calls))

    def test_failure_to_stop_a_legacy_service_is_fatal(self):
        def run(arguments, **kwargs):
            if 'disable' in arguments:
                return subprocess.CompletedProcess(arguments, 1)
            output = ('/home/pi/orion/voice/.venv/bin/orion-voice listen-worker'
                      if 'show' in arguments else 'old-voice.service enabled\n')
            return subprocess.CompletedProcess(arguments, 0, output, '')
        with patch.object(retire.subprocess, 'run', run):
            with self.assertRaisesRegex(RuntimeError, 'old-voice.service.*refusing to replace'):
                retire.retire_services(Path('/home/pi/orion'))

    def test_unavailable_user_manager_is_only_skipped_without_local_legacy_units(self):
        def run(arguments, **kwargs):
            return subprocess.CompletedProcess(arguments, int('--user' in arguments), '', 'No bus')
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            units = home / '.config/systemd/user'
            units.mkdir(parents=True)
            with patch.object(retire.Path, 'home', return_value=home), patch.object(retire.subprocess, 'run', run):
                retire.retire_services(Path('/home/pi/orion'))
                (units / 'legacy@.service').write_text('ExecStart=/home/pi/orion/voice/.venv/bin/orion-voice wake-worker')
                with self.assertRaisesRegex(RuntimeError, 'Start the user systemd manager'):
                    retire.retire_services(Path('/home/pi/orion'))

    def test_known_models_archived_and_rustpotter_custom_files_preserved(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            models = root / 'voice/models'; models.mkdir(parents=True)
            for relative in retire.MODEL_PATHS:
                source = models / relative; source.parent.mkdir(parents=True, exist_ok=True); source.write_text('legacy')
            (models / 'en_US-ryan-medium.onnx.json').write_text(json.dumps(dict(audio={}, phoneme_id_map={})))
            (models / 'en_US-ryan-medium.onnx').write_text('piper')
            reference = models / 'wake/hey_orion_reference.rpw'; reference.write_text('rustpotter')
            custom = models / 'custom.onnx'; custom.write_text('custom')
            moved = retire.archive_models(root, root / 'backup')
            self.assertEqual(len(moved), 5)
            self.assertEqual(reference.read_text(), 'rustpotter')
            self.assertEqual(custom.read_text(), 'custom')
            self.assertEqual(retire.archive_models(root, root / 'second'), [])

    def test_symlinked_downloads_are_not_followed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); models = root / 'voice/models'; models.mkdir(parents=True)
            outside = root / 'keep'; outside.write_text('keep')
            vad = models / 'vad'; vad.mkdir()
            (vad / 'silero_vad.onnx').symlink_to(outside)
            self.assertEqual(retire.archive_models(root, root / 'backup'), [])
            self.assertEqual(outside.read_text(), 'keep')


class InstallerTests(unittest.TestCase):
    def exercise(self, fail_sync=False, packages_installed=False):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary); root = base / 'checkout'; home = base / 'user'; binaries = base / 'bin'
            binaries.mkdir(); (root / 'scripts').mkdir(parents=True); (root / 'voice/.venv').mkdir(parents=True)
            (root / 'voice/.venv/old-stack').write_text('legacy environment')
            # A test-only cleanup stand-in; actual cleanup is covered above.
            (root / 'scripts/retire_pi_voice.py').write_text('print("retired fixture stack")')
            bootstrap = home / '.local/share/orion/uv-bootstrap/bin'; bootstrap.mkdir(parents=True)
            log = base / 'calls'
            fake = '''#!/usr/bin/env python3
import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
with open(os.environ['ORION_TEST_LOG'], 'a') as log: log.write(json.dumps([name, *sys.argv[1:]])+'\\n')
if name == 'uname': print('Linux' if sys.argv[1] == '-s' else 'aarch64')
if name == 'sudo' and '-n' in sys.argv: sys.exit('non-interactive sudo refuses the password prompt')
if name == 'dpkg-query':
 print('install ok installed' if os.environ['ORION_TEST_PACKAGES'] == '1' else 'unknown ok not-installed')
if name == 'uv':
 root = pathlib.Path(sys.argv[sys.argv.index('--project') + 1])
 (root / '.venv/bin').mkdir(parents=True)
 (root / '.venv/bin/python').symlink_to(os.environ['ORION_TEST_PYTHON'])
 sys.exit(int(os.environ['ORION_TEST_FAIL']))
'''
            for name in ['sudo', 'systemctl', 'uname', 'cargo', 'dpkg-query']:
                file = binaries / name; file.write_text(fake); file.chmod(0o755)
            for name in ['python', 'uv']:
                file = bootstrap / name; file.write_text(fake); file.chmod(0o755)
            env = {**os.environ, 'PATH': str(binaries) + ':' + os.environ['PATH'],
                   'ORION_TEST_LOG': str(log), 'ORION_TEST_FAIL': str(int(fail_sync)),
                   'ORION_TEST_PACKAGES': str(int(packages_installed)),
                   'ORION_TEST_PYTHON': str(bootstrap / 'python')}
            result = subprocess.run(['bash', str(SCRIPTS / 'install_pi_voice.sh'), str(root), str(home)],
                                    env=env, text=True, capture_output=True)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            self.assertEqual(result.returncode, int(fail_sync), result.stderr)
            sync = next(call for call in calls if call[0] == 'uv')
            self.assertIn('--locked', sync); self.assertNotIn('--extra', sync)
            apt = [call for call in calls if call[0] == 'sudo' and 'apt-get' in call]
            if packages_installed:
                self.assertEqual(apt, [])
            else:
                self.assertTrue(any('alsa-utils' in call for call in apt))
            self.assertIn(['sudo', '-v'], calls)
            self.assertTrue(all('-n' not in call for call in calls if call[0] == 'sudo'))
            if fail_sync:
                self.assertEqual((root / 'voice/.venv/old-stack').read_text(), 'legacy environment')
                self.assertTrue(list(home.glob('.local/share/orion/backups/voice-*/failed-venv')))
            else:
                self.assertFalse((root / 'voice/.venv/old-stack').exists())
                self.assertTrue(list(home.glob('.local/share/orion/backups/voice-*/venv/old-stack')))
                self.assertTrue(any('unittest' in call for call in calls))
                self.assertTrue(any('--no-default-groups' in call for call in calls))

    def test_success_replaces_old_environment_and_validates_new_stack(self): self.exercise()
    def test_installed_packages_skip_privileged_package_manager(self): self.exercise(packages_installed=True)
    def test_failure_restores_old_environment_without_restarting_workers(self): self.exercise(True)


if __name__ == '__main__':
    unittest.main()
