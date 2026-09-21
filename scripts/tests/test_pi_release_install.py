"""Configuration preservation and fault-injected release/rollback transactions."""
import copy
import json
import io
import subprocess
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
import install_pi_voice_stack as installer
from pi_service_config import SERVICES, effective_start, read_env, render_plan


class Fixture(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.home = self.base / 'user'; self.home.mkdir()
        self.root = self.home / '.local/share/orion/voice-stack'; self.root.mkdir(parents=True)
        self.release = self.root / 'releases/new'; self.release.mkdir(parents=True)
        self.units = self.base / 'units'; self.units.mkdir()
        self.project = self.home / 'dev/orion'; self.project.mkdir(parents=True)
        shutil.copytree(SCRIPTS / 'systemd', self.release / 'scripts/systemd')
        (self.release / 'release.json').write_text('{"revision":"new"}')
        self.env = self.home / '.config/orion/voice-stack.env'; self.env.parent.mkdir(parents=True)

    def plan(self):
        return render_plan(self.release, self.root, self.project, self.home, 'pi', self.units)

    def write(self, path, text):
        path.parent.mkdir(parents=True, exist_ok=True); path.write_text(text)

    def installed_units(self):
        for path, text in self.plan().items():
            self.write(path, text.replace(str(self.release), '/old/release'))


class ConfigurationTests(Fixture):
    def test_preserves_custom_environment_and_only_changes_release_fields(self):
        self.env.write_text('# operator tuning\nORION_ASR_THREADS="2"\nORION_TTS_THREADS=4\n'
            'ORION_ASR_CONTEXT="Hey Orion and Ada"\nORION_ASR_MODEL_DIR="/custom/qwen"\n'
            'HF_HOME=/custom/cache\nHF_HUB_OFFLINE=0\nORION_STUDIO_CODEX_BIN=/custom/codex\n'
            'ORION_STUDIO_TTS_MODEL=pocket-int8\nORION_STUDIO_VOICE_PYTHON=/old/python\n'
            'ORION_PROJECT_ROOT=/old/root\nORION_RELEASE_REVISION=old\nCUSTOM=value')
        old = read_env(self.env.read_text()); result = self.plan()[self.env]; new = read_env(result)
        for key, value in old.items():
            if key not in ('ORION_STUDIO_VOICE_PYTHON', 'ORION_PROJECT_ROOT', 'ORION_RELEASE_REVISION'):
                self.assertEqual(new[key], value, key)
        self.assertEqual(new['ORION_STUDIO_VOICE_PYTHON'], str(self.release / 'speech/.venv/bin/python'))
        self.assertEqual(new['ORION_PROJECT_ROOT'], str(self.release))
        self.assertEqual(new['ORION_RELEASE_REVISION'], 'new')
        self.assertIn('# operator tuning\n', result)
        self.assertIn('CUSTOM=value\n', result)
        self.assertEqual(self.env.read_text().splitlines()[-1], 'CUSTOM=value')

    def test_all_override_commands_move_and_tuning_survives(self):
        self.installed_units()
        rest = self.units / 'oriond.service.d/50-rest-timeout.conf'
        listener = self.units / 'orion-listener.service.d/40-pi-voice.conf'
        gateway = self.units / 'orion-studio-gateway.service.d/40-pi-voice.conf'
        self.write(rest, '[Service]\nExecStart=\nExecStart=/old/release/runtime/target/release/oriond --serve --backend hardware --rest-after-seconds 1800 --socket /tmp/custom.sock\n')
        self.write(listener, '[Service]\nWorkingDirectory=/old/release/voice\nEnvironment=ORION_CAPTURE_GAIN_DB=21\nEnvironment=ORION_VAD_MODEL=/custom/silero.onnx\nExecStart=\nExecStart=/old/release/voice/.venv/bin/orion-listener --local-processor --threshold 0.42 --mic-spacing ${ORION_MIC_SPACING}\n')
        self.write(gateway, '[Service]\nExecStart=\nExecStart=/usr/bin/python3 /old/release/orion_studio/gateway.py serve --project-root '+str(self.project)+' --port 7555 --trajectory-compiler /old/release/runtime/target/release/orion-trajectory\n')
        plan = self.plan()
        self.assertIn('--rest-after-seconds 1800 --socket /tmp/custom.sock', plan[rest])
        self.assertIn('--threshold 0.42 --mic-spacing ${ORION_MIC_SPACING}', plan[listener])
        self.assertIn('ORION_CAPTURE_GAIN_DB=21', plan[listener])
        self.assertIn('ORION_VAD_MODEL=/custom/silero.onnx', plan[listener])
        self.assertIn('--project-root '+str(self.project)+' --port 7555', plan[gateway])
        for path, text in plan.items():
            self.assertNotIn('/old/release/', text, str(path))
        self.assertIn('WorkingDirectory='+str(self.project), plan[self.units / 'oriond.service'])
        self.assertIn('WorkingDirectory='+str(self.project), plan[self.units / 'orion-studio-gateway.service'])

    def test_later_listener_override_gets_local_processor_and_keeps_its_threshold(self):
        self.installed_units()
        path = self.units / 'orion-listener.service.d/99-local.conf'
        self.write(path, '[Service]\nExecStart=\nExecStart=/old/release/voice/.venv/bin/orion-listener --threshold 0.47\n')
        result = self.plan()[path]
        self.assertIn('--threshold 0.47 --local-processor', result)
        self.assertNotIn('--threshold 0.45', result)

    def test_saved_preferences_and_audio_calibration_are_outside_write_set(self):
        for name in ('voice-settings.json', 'microphone.json', 'servo_calibration.json', 'voice.env', 'studio-token'):
            self.write(self.env.parent / name, 'operator settings')
        plan = self.plan()
        for name in ('voice-settings.json', 'microphone.json', 'servo_calibration.json', 'voice.env', 'studio-token'):
            path = self.env.parent / name
            self.assertNotIn(path, plan)
            self.assertEqual(path.read_text(), 'operator settings')
        self.assertNotIn(self.home / '.local/share/orion/SOUL.md', plan)
        self.assertNotIn(self.home / '.local/share/orion/MEMORY.md', plan)

    def test_unsupported_env_and_symlinked_overrides_fail_without_writes(self):
        self.env.write_text('BROKEN="unterminated')
        with self.assertRaises(ValueError): self.plan()
        self.env.unlink()
        path = self.units / 'oriond.service.d/custom.conf'; path.parent.mkdir()
        path.symlink_to(self.env)
        with self.assertRaises(ValueError): self.plan()


class RestRuntimeTests(unittest.TestCase):
    def exercise(self, failures=None, torque_after=False):
        system = installer.System()
        self.calls = []
        statuses = iter([True, torque_after])
        failures = failures or {}

        def run(*args, **kwargs):
            if args[0] == 'systemctl':
                return subprocess.CompletedProcess(args, 0, '42\n')
            self.assertEqual(args[:3], ('/proc/42/exe', '--socket', '/tmp/custom.sock'))
            command = args[3]
            self.calls.append(command)
            if command in failures:
                code, output = failures[command]
                raise subprocess.CalledProcessError(code, args, output=output)
            if command == '--goto':
                self.assertEqual(args[4:], ('rest', '--duration', '3.0', '--wait'))
            output = json.dumps({'torque_enabled': next(statuses)}) if command == '--status' else ''
            return subprocess.CompletedProcess(args, 0, output)

        cmdline = b'oriond\0--serve\0--socket\0/tmp/custom.sock\0'
        with patch.object(system, 'run', side_effect=run), \
             patch.object(installer.Path, 'read_bytes', return_value=cmdline):
            system.rest_runtime()

    def test_active_and_already_inactive_playback_both_reach_confirmed_rest(self):
        for scene in (False, True):
            for speech in (False, True):
                with self.subTest(scene_inactive=scene, speech_inactive=speech):
                    failures = {}
                    if scene: failures['--stop-scene'] = (3, '{"ok":false,"error":"No scene is active."}')
                    if speech: failures['--stop-speech'] = (3, '{"ok":false,"error":"No speech run is active."}')
                    self.exercise(failures)
                    self.assertEqual(self.calls, ['--status', '--stop-scene', '--stop-speech', '--stop', '--goto', '--disable', '--status'])

    def test_other_stop_failures_abort_before_moving_or_disabling(self):
        for command in ('--stop-scene', '--stop-speech', '--stop'):
            for code, output in [(3, '{"ok":false,"error":"Device failed"}'),
                                 (1, '{"ok":false,"error":"No scene is active."}'),
                                 (3, 'invalid JSON'), (3, ''), (3, 'null')]:
                with self.subTest(command=command, code=code, output=output):
                    with self.assertRaises(subprocess.CalledProcessError):
                        self.exercise({command: (code, output)})
                    self.assertNotIn('--goto', self.calls)
                    self.assertNotIn('--disable', self.calls)

    def test_already_stationary_motion_still_requires_rest_and_torque_off(self):
        self.exercise({'--stop': (3, '{"ok":false,"command":"stop","error":"no movement is active"}')})
        self.assertEqual(self.calls, ['--status', '--stop-scene', '--stop-speech', '--stop', '--goto', '--disable', '--status'])
        with self.assertRaisesRegex(RuntimeError, 'torque-off was not confirmed'):
            self.exercise({'--stop': (3, '{"ok":false,"command":"stop","error":"no movement is active"}')}, torque_after=True)

    def test_failed_rest_never_disables_and_failed_disable_aborts(self):
        for command in ('--goto', '--disable'):
            with self.subTest(command=command):
                with self.assertRaises(subprocess.CalledProcessError):
                    self.exercise({command: (4, 'failed')})
                if command == '--goto': self.assertNotIn('--disable', self.calls)

    def test_torque_must_be_explicitly_confirmed_off(self):
        for value in (True, None):
            with self.subTest(torque=value):
                with self.assertRaisesRegex(RuntimeError, 'torque-off was not confirmed'):
                    self.exercise(torque_after=value)


class FakeSystem:
    """Model unit existence separately from enablement to catch dangling wants links."""
    def __init__(self, units, existing=True):
        self.units = units
        self.states = {name: {'active': existing, 'enabled': 'enabled' if existing else 'not-found'} for name in SERVICES}
        self.calls = []
        self.failure = None
        self.fail_restore = False
        self.initial_bytes = {}

    def state(self, name): return copy.deepcopy(self.states[name])
    def stop(self, name):
        self.calls.append(('stop', name)); self.states[name]['active'] = False
    def start(self, name):
        self.calls.append(('start', name))
        if self.failure == 'start':
            self.failure = None; raise RuntimeError('Injected start failure')
        self.states[name]['active'] = True
        if name == 'oriond' and self.states['orion-listener']['enabled'] != 'not-found':
            self.states['orion-listener']['active'] = True
    def enable(self, name, state):
        self.calls.append(('enable', name, state))
        if self.states[name]['enabled'] == 'not-found':
            raise AssertionError('Enablement must change while the unit still exists')
        self.states[name]['enabled'] = 'disabled' if state == 'not-found' else state
    def reload(self):
        self.calls.append(('reload',))
        for name, state in self.states.items():
            exists = (self.units / (name+'.service')).exists()
            if not exists:
                if state['enabled'] == 'enabled': raise AssertionError('Dangling enablement after unit removal')
                state['enabled'] = 'not-found'
            elif state['enabled'] == 'not-found': state['enabled'] = 'disabled'
    def write(self, path, data, mode):
        self.calls.append(('write', str(path)))
        if self.fail_restore and data == self.initial_bytes.get(path): raise RuntimeError('Cannot restore')
        if self.failure == 'write':
            self.failure = None; raise RuntimeError('Injected write failure')
        path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(data); path.chmod(mode)
    def remove(self, path): path.unlink(missing_ok=True)
    def rest_runtime(self):
        self.calls.append(('rest',))
        if self.failure == 'rest': raise RuntimeError('Unsafe rest')
    def ready(self, release, home):
        self.calls.append(('ready',))
        if self.failure in ('ready', 'interrupt'):
            failure = self.failure; self.failure = None
            if failure == 'interrupt': raise KeyboardInterrupt()
            raise RuntimeError('Injected readiness failure')


class TransactionTests(Fixture):
    def setUp(self):
        super().setUp()
        self.installed_units()
        self.system = FakeSystem(self.units)
        self.system.states['orion-studio-gateway']['enabled'] = 'disabled'
        self.system.states['orion-listener']['active'] = False
        self.manifest = self.root / 'installation.json'
        self.manifest.write_text(json.dumps({'release': '/old/release', 'files': [{'backup': None}]}))
        self.original = {p: p.read_bytes() for p in [*self.plan(), self.manifest]}
        self.modes = {p: p.stat().st_mode & 0o777 for p in self.original}
        self.original_states = copy.deepcopy(self.system.states)
        self.system.initial_bytes = self.original

    def activate(self): return installer.activate(self.root, self.release, self.home, self.plan(), self.system)

    def assert_restored(self):
        for path, data in self.original.items():
            self.assertEqual(path.read_bytes(), data, str(path))
            self.assertEqual(path.stat().st_mode & 0o777, self.modes[path])
        self.assertEqual(self.system.states, self.original_states)
        self.assertFalse((self.root / 'pending-installation.json').exists())

    def test_write_start_health_and_interrupt_failures_restore_immediate_install(self):
        for failure in ('write', 'start', 'ready', 'interrupt'):
            with self.subTest(failure=failure):
                self.system.failure = failure
                with self.assertRaises((RuntimeError, KeyboardInterrupt)): self.activate()
                self.assert_restored()

    def test_unsafe_rest_does_not_stop_runtime_or_change_config(self):
        self.system.failure = 'rest'
        with self.assertRaisesRegex(RuntimeError, 'Unsafe rest'): self.activate()
        self.assertNotIn(('stop', 'oriond'), self.system.calls)
        self.assert_restored()

    def test_success_saves_one_recent_update_and_manual_rollback_is_one_level(self):
        first = self.activate()
        # Operator adjusts tuning after first deployment; the next rollback must capture it.
        self.env.write_text(self.env.read_text().replace('ORION_TTS_THREADS="3"', 'ORION_TTS_THREADS="2"'))
        prior_env = self.env.read_bytes()
        prior_release = self.release
        self.release = self.root / 'releases/second'; shutil.copytree(prior_release, self.release)
        (self.release / 'release.json').write_text('{"revision":"second"}')
        second = self.activate()
        self.assertFalse(first.exists())
        self.assertEqual(list((self.root / 'transactions').iterdir()), [second])
        installer.rollback(self.root, self.system)
        self.assertEqual(self.env.read_bytes(), prior_env)
        self.assertEqual(json.loads(self.manifest.read_text())['release'], str(prior_release))
        calls = len(self.system.calls)
        installer.rollback(self.root, self.system)
        self.assertEqual(len(self.system.calls), calls)

    def test_recovery_finishes_metadata_after_interruption_following_restore(self):
        first = self.activate()
        previous = self.release
        self.release = self.root / 'releases/second'; shutil.copytree(previous, self.release)
        (self.release / 'release.json').write_text('{"revision":"second"}')
        second = self.activate()
        installer.atomic_json(self.root / 'pending-installation.json', {'transaction': str(second)})
        data = json.loads((second / 'state.json').read_text())
        installer.restore(second, data, self.system)
        self.assertFalse(first.exists())
        self.assertEqual(json.loads(self.manifest.read_text())['rollback'], str(first))
        calls = len(self.system.calls)
        installer.rollback(self.root, self.system)
        self.assertEqual(len(self.system.calls), calls)
        self.assertEqual(json.loads(self.manifest.read_text())['rollback'], str(second))
        self.assertFalse((self.root / 'pending-installation.json').exists())

    def test_failed_update_keeps_previous_rollback(self):
        first = self.activate()
        previous_manifest = self.manifest.read_bytes()
        self.release = self.root / 'releases/failed'; self.release.mkdir()
        (self.release / 'release.json').write_text('{"revision":"failed"}')
        self.system.failure = 'ready'
        with self.assertRaises(RuntimeError): self.activate()
        self.assertEqual(self.manifest.read_bytes(), previous_manifest)
        self.assertTrue(first.exists())

    def test_failed_recovery_keeps_journal_and_blocks_new_update(self):
        self.system.failure = 'ready'; self.system.fail_restore = True
        with self.assertRaisesRegex(RuntimeError, 'Recovery could not finish'): self.activate()
        pending = self.root / 'pending-installation.json'
        self.assertTrue(pending.exists())
        with self.assertRaisesRegex(RuntimeError, 'interrupted deployment'): self.activate()
        self.system.fail_restore = False
        installer.rollback(self.root, self.system)
        self.assert_restored()

    def test_legacy_original_install_manifest_is_never_used_as_update_rollback(self):
        with self.assertRaisesRegex(RuntimeError, 'legacy pre-installation'): installer.rollback(self.root, self.system)
        self.assertEqual(self.system.calls, [])

    def test_first_install_failure_removes_units_and_enablement(self):
        for path in self.plan(): path.unlink()
        self.manifest.unlink()
        self.system = FakeSystem(self.units, existing=False)
        self.system.failure = 'ready'
        with self.assertRaises(RuntimeError): self.activate()
        self.assertFalse(self.manifest.exists())
        for path in self.plan(): self.assertFalse(path.exists(), str(path))
        self.assertTrue(all(state == {'active': False, 'enabled': 'not-found'} for state in self.system.states.values()))

    def test_release_reactivation_is_rejected_before_stopping(self):
        self.activate(); self.system.calls.clear()
        with self.assertRaisesRegex(RuntimeError, 'already installed'): self.activate()
        self.assertEqual(self.system.calls, [])


class ReadinessTests(Fixture):
    def test_processes_alone_are_not_ready_and_only_matching_release_is_accepted(self):
        for fault in (None, 'old-service', 'old-runtime', 'wrong-gateway', 'no-asr', 'no-tts', 'no-agent'):
            with self.subTest(fault=fault):
                status = dict(coordinator_running=True, error=None, project_root=str(self.release), revision='new', pid=42)
                event = dict(type='ready', asr=dict(provider='qwen3-asr'), tts=dict(provider='pocket-tts'), agent=dict(provider='codex'))
                revision, gateway_pid = 'new', 42
                if fault == 'old-service': status['project_root'] = '/old/release'
                if fault == 'old-runtime': revision = 'old'
                if fault == 'wrong-gateway': gateway_pid = 77
                for key in ('asr', 'tts', 'agent'):
                    if fault == 'no-'+key: event[key] = {}
                system = installer.System()
                self.write(self.home / '.config/orion/custom-token', 'fixture-token')
                cmdline = '\0'.join(['python3', 'gateway.py', '--port', '7555', '--socket', '/tmp/custom.sock', '--token-file', str(self.home / '.config/orion/custom-token'), '']).encode()
                def run(*args, **kwargs):
                    if args[0] == 'systemctl': return subprocess.CompletedProcess(args, 0, '100\n')
                    self.assertIn('/tmp/custom.sock', args)
                    return subprocess.CompletedProcess(args, 0, json.dumps(dict(build_revision=revision)))
                def request(req, **kwargs):
                    self.assertEqual(req.full_url, 'http://127.0.0.1:7555/api/v2/voice/request')
                    self.assertEqual(req.headers['Authorization'], 'Bearer fixture-token')
                    return io.BytesIO(json.dumps(dict(pid=gateway_pid)).encode())
                with patch.object(system, 'state', return_value=dict(active=True)), \
                     patch.object(system, 'run', side_effect=run), \
                     patch.object(installer.Path, 'read_bytes', return_value=cmdline), \
                     patch.object(installer, 'control', side_effect=[status, dict(events=[event])]), \
                     patch.object(installer.time, 'monotonic', side_effect=[0, 0, 2]), \
                     patch.object(installer.time, 'sleep'), \
                     patch.object(installer.urllib.request, 'urlopen', side_effect=request):
                    if fault:
                        with self.assertRaisesRegex(RuntimeError, 'did not become ready'):
                            system.ready(self.release, self.home, timeout=1)
                    else:
                        system.ready(self.release, self.home, timeout=1)

    def test_preflight_uses_saved_env_including_project_override_without_mutation(self):
        self.env.write_text('ORION_PROJECT_ROOT=/old/root\nORION_STUDIO_CODEX_BIN=/custom/codex\nORION_TTS_THREADS=2\n')
        for name in ('runtime/target/release/oriond', 'runtime/target/release/orion-trajectory',
                     'orion-service/target/release/orion-service', 'speech/.venv/bin/python', 'voice/.venv/bin/orion-listener'):
            self.write(self.release / name, 'fixture')
        for name in ('servo_calibration.json', 'studio-token'):
            self.write(self.env.parent / name, 'fixture')
        system = installer.System()
        before = self.env.read_bytes()
        with patch.object(system, 'run') as run:
            installer.preflight(self.release, self.home, self.plan(), system)
        self.assertEqual(run.call_args_list[0].args, ('/custom/codex', 'login', 'status'))
        self.assertEqual(run.call_args.kwargs['env']['ORION_TTS_THREADS'], '2')
        self.assertEqual(run.call_args.kwargs['env']['ORION_PROJECT_ROOT'], str(self.release))
        self.assertEqual(self.env.read_bytes(), before)

    def test_standalone_listener_installer_refuses_installed_full_stack(self):
        (self.root / 'installation.json').write_text('{}')
        result = subprocess.run(['bash', SCRIPTS / 'install_pi_voice.sh', str(self.project), str(self.home)], capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('complete update', result.stderr)


if __name__ == '__main__':
    unittest.main()
