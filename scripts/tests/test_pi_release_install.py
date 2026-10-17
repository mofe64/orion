"""Configuration preservation and fault-injected release/rollback transactions."""
import copy
import importlib.util
import os
import json
import io
import shlex
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
import pi_service_config
from release_assets import ASSET_DIRECTORIES, remove_assets
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
        for name in ('home_assistant_install.py', 'release-home-assistant.json'):
            shutil.copy2(SCRIPTS / name, self.release / 'scripts' / name)
        shutil.copytree(SCRIPTS.parent / 'hardware/profiles', self.release / 'hardware/profiles')
        (self.release / 'release.json').write_text('{"revision":"new"}')
        self.env = self.home / '.config/orion/voice-stack.env'; self.env.parent.mkdir(parents=True)

    def plan(self):
        return render_plan(self.release, self.root, self.project, self.home, 'pi', self.units, system_root=self.base / 'system')

    def write(self, path, text):
        path.parent.mkdir(parents=True, exist_ok=True); path.write_text(text)

    def installed_units(self):
        for path, text in self.plan().items():
            self.write(path, text.replace(str(self.release), '/old/release'))


class ConfigurationTests(Fixture):
    def test_release_updates_use_the_selected_executable_at_different_install_paths(self):
        template = self.release
        for relative in ('home/alice/.local/share/orion/voice-stack', 'opt/custom-orion', 'srv/robots/bob'):
            with self.subTest(root=relative):
                root = self.base / relative
                home = root.parent / 'settings-home'; home.mkdir(parents=True, exist_ok=True)
                project = root.parent / 'assets'; project.mkdir(exist_ok=True)
                units = root.parent / 'systemd'; units.mkdir(exist_ok=True)
                first = root / 'releases/first'; shutil.copytree(template, first)
                first_plan = render_plan(first, root, project, home, 'pi', units, system_root=self.base / 'system')
                for path, text in first_plan.items():
                    self.write(path, text)
                second = root / 'releases/second'; shutil.copytree(template, second)
                (second / 'release.json').write_text('{"revision":"second"}')
                plan = render_plan(second, root, project, home, 'pi', units, system_root=self.base / 'system')
                unit = plan[units / 'orion-voice-stack.service']
                self.assertIn('ExecStart=' + str(second / 'orion-service/target/release/orion-service') + ' serve', unit)
                self.assertIn('WorkingDirectory=' + str(second), unit)
                self.assertNotIn(str(first), unit)
                gateway_unit = plan[units / 'orion-studio-gateway.service']
                self.assertIn(str(second / 'orion_studio/gateway.py'), gateway_unit)
                self.assertIn('--project-root ' + str(project), gateway_unit)
                self.assertNotIn(str(first), gateway_unit)

    def test_v2_services_use_catalog_asset_paths_after_release_assets_are_removed(self):
        (self.release / 'release.json').write_text('{"revision":"new","hardware":"v2"}')
        expected = {
            '--poses': 'motion/config/v2/poses.yaml',
            '--motions': 'motion/motions/v2',
            '--scenes': 'scenes/v2',
            '--cues': 'audio/cues',
        }
        self.assertEqual(set(ASSET_DIRECTORIES), {
            directory for directory in ASSET_DIRECTORIES
            if any(path == directory or path.startswith(directory + '/') for path in expected.values())})
        builtins = ('motion/config/v2/poses.yaml', 'motion/motions/v2/idle.yaml',
                    'scenes/v2/scene.yaml', 'audio/cues/cue.wav')
        for relative in builtins:
            self.write(self.release / relative, 'built-in asset')
            self.write(self.project / relative, 'catalog asset')
        remove_assets(self.release)
        self.assertTrue(all(not (self.release / relative).exists() for relative in builtins))
        unit = self.units / 'oriond.service'
        override = self.units / 'oriond.service.d/99-local.conf'
        for existing_override in (False, True):
            with self.subTest(existing_override=existing_override):
                if existing_override:
                    arguments = ' '.join(f'{flag} /old/release/{relative}' for flag, relative in expected.items())
                    self.write(override, '[Service]\nExecStart=\nExecStart=/old/release/runtime/target/release/oriond --serve ' + arguments + '\n')
                plan = self.plan()
                arguments = shlex.split(effective_start([(p, plan[p]) for p in (unit, override) if p in plan]))
                for flag, relative in expected.items():
                    self.assertEqual(arguments[arguments.index(flag) + 1], str(self.project / relative))
                gateway = plan[self.units / 'orion-studio-gateway.service']
                self.assertIn('--project-root ' + str(self.project), gateway)
                for path, value in plan.items():
                    if path.suffix in ('.service', '.conf'):
                        for directory in ASSET_DIRECTORIES:
                            self.assertNotIn(str(self.release / directory), value, str(path))
                for relative in builtins:
                    self.assertEqual((self.project / relative).read_text(), 'catalog asset')

    def test_rendered_gateway_starts_with_catalog_only_root_and_release_compiler(self):
        (self.release / 'release.json').write_text('{"revision":"new","hardware":"v2"}')
        self.write(self.project / 'motion/config/v2/poses.yaml', 'poses: {}')
        (self.project / 'scenes/v2').mkdir(parents=True)
        gateway_file = self.release / 'orion_studio/gateway.py'
        gateway_file.parent.mkdir(parents=True)
        for name in ('gateway.py', 'system_metrics.py', 'home_assistant.py'):
            shutil.copy2(SCRIPTS.parent / 'orion_studio' / name, gateway_file.parent / name)
        compiler = self.release / 'runtime/target/release/orion-trajectory'
        self.write(compiler, '#!/bin/sh\nexit 0\n'); compiler.chmod(0o755)
        token = self.home / '.config/orion/studio-token'
        self.write(token, 'test-pairing-secret-' * 3); token.chmod(0o600)
        self.assertFalse((self.project / 'AGENTS.md').exists())
        self.assertFalse((self.project / 'runtime').exists())
        spec = importlib.util.spec_from_file_location('release_gateway', gateway_file)
        gateway = importlib.util.module_from_spec(spec)
        with patch.object(sys, 'path', [str(gateway_file.parent), *sys.path]):
            spec.loader.exec_module(gateway)
        start = effective_start([(p, text) for p, text in self.plan().items()
                                 if p.name == 'orion-studio-gateway.service'])
        arguments = shlex.split(start)[1:]
        for explicit_compiler in (True, False):
            with self.subTest(explicit_compiler=explicit_compiler):
                argv = arguments.copy()
                if not explicit_compiler:
                    index = argv.index('--trajectory-compiler'); del argv[index:index+2]
                original = gateway.OrionGateway
                def construct(*args, **kwargs):
                    instance = original(*args, **kwargs)
                    self.assertEqual(instance._trajectory_compiler_path().resolve(), compiler.resolve())
                    return instance
                with patch.object(sys, 'argv', argv), patch.object(gateway, 'OrionGateway', side_effect=construct), \
                     patch.object(gateway, 'GatewayHTTPServer') as server, patch.dict(os.environ):
                    os.environ.pop('ORION_TRAJECTORY_COMPILER', None)
                    gateway.main()
                server.return_value.serve_forever.assert_called_once()
        for missing in (self.project / 'scenes/v2', self.project / 'motion/config/v2/poses.yaml'):
            hidden = missing.with_name(missing.name + '.hidden'); missing.rename(hidden)
            try:
                with patch.object(sys, 'argv', arguments), self.assertRaisesRegex(SystemExit, 'project root'):
                    gateway.main()
            finally:
                hidden.rename(missing)

    def test_v2_camera_geometry_and_detector_are_loaded_from_release_including_overrides(self):
        (self.release / 'release.json').write_text('{"revision":"new","hardware":"v2"}')
        self.write(self.release / 'vision/orion_vision/detect_hands.py', '# module present')
        self.installed_units()
        override = self.units / 'oriond.service.d/99-local.conf'
        self.write(override, '[Service]\nExecStart=\nExecStart=/old/release/runtime/target/release/oriond '
                   '--serve --camera-urdf /old/geometry.urdf --socket /tmp/custom.sock\n')
        plan = self.plan()
        start = effective_start([(override, plan[override])])
        self.assertIn('--camera-urdf ' + str(self.release / 'description/urdf/orion-v2.urdf'), start)
        self.assertEqual(start.count('--camera-urdf'), 1)
        self.assertIn('--palm-model ' + str(self.release / 'vision/models/palm_detection_mediapipe_2023feb.onnx'),
                      plan[self.units / 'orion-vision.service'])
        self.assertIn('--socket /tmp/custom.sock', start)
        self.assertIn('--face-model ' + str(self.release / 'vision/models/face_detection_yunet_2023mar.onnx'),
                      plan[self.units / 'orion-vision.service'])

    def test_camera_unit_is_profile_gated_and_does_not_change_privacy_preference(self):
        privacy = self.env.parent / 'camera.json'
        privacy.write_text('{"camera_enabled":false}')
        tracking = self.env.parent / 'tracking.json'
        tracking.write_text('{"tracking_mode":"off"}')
        self.assertNotIn(self.units / 'orion-vision.service', self.plan())
        (self.release / 'release.json').write_text('{"revision":"new","hardware":"v2"}')
        plan = self.plan()
        unit = plan[self.units / 'orion-vision.service']
        self.assertIn(str(self.release / 'vision/.venv/bin/orion-vision'), unit)
        self.assertIn(str(self.release / 'hardware/profiles/v2.json'), unit)
        for setting in ('Nice=10', 'CPUQuota=150%', 'SupplementaryGroups=video',
                        'Environment=OMP_NUM_THREADS=1', 'Environment=OPENBLAS_NUM_THREADS=1'):
            self.assertIn(setting, unit)
        self.assertNotIn(privacy, plan)
        self.assertEqual(privacy.read_text(), '{"camera_enabled":false}')
        self.assertNotIn(tracking, plan)
        self.assertEqual(tracking.read_text(), '{"tracking_mode":"off"}')
        profile_path = self.release / 'hardware/profiles/v2.json'
        selected = json.loads(profile_path.read_text())
        selected['camera']['enabled'] = False
        profile_path.write_text(json.dumps(selected))
        self.assertNotIn(self.units / 'orion-vision.service', self.plan())

    def test_vision_readiness_accepts_privacy_off_but_requires_capture_when_enabled(self):
        from unittest.mock import MagicMock
        for status, body, accepted in ((503, {"error": {"code": "camera_off"}}, True),
                                      (200, {"capture_boottime_ns": 123, "detection": {"error": None, "inferences": 1}}, True),
                                      (200, {"capture_boottime_ns": 123, "detection": {"error": None, "inferences": 0}}, False),
                                      (200, {"capture_boottime_ns": 123, "tracking_mode": "off",
                                             "detection": {"error": None, "inferences": 0}}, True),
                                      (200, {"capture_boottime_ns": 123, "tracking_mode": "idle",
                                             "detection": {"error": None, "inferences": 1}}, True),
                                      (200, {"capture_boottime_ns": 123, "tracking_mode": "idle",
                                             "detection": {"error": None, "inferences": 0}}, False),
                                      (200, {"capture_boottime_ns": None, "tracking_mode": "off"}, False),
                                      (200, {"capture_boottime_ns": 123}, False),
                                      (200, {"capture_boottime_ns": None}, False),
                                      (200, {"capture_boottime_ns": 123, "detection": {"error": "Model missing"}}, False),
                                      (503, {"error": {"code": "camera_unavailable"}}, False)):
            with self.subTest(status=status, body=body):
                response = MagicMock(status=status)
                response.read.return_value = json.dumps(body).encode()
                with patch.object(installer.http.client, 'HTTPConnection') as connection, \
                     patch.object(installer.socket, 'socket'):
                    connection.return_value.getresponse.return_value = response
                    if accepted:
                        installer.check_vision_health()
                    else:
                        with self.assertRaises(RuntimeError):
                            installer.check_vision_health()
                    connection.return_value.close.assert_called_once()

    def test_new_listener_uses_active_threshold_on_both_hardware_versions(self):
        for hardware in ('v1', 'v2'):
            with self.subTest(hardware=hardware):
                (self.release / 'release.json').write_text(json.dumps({'revision': 'new', 'hardware': hardware}))
                result = self.plan()[self.units / 'orion-listener.service']
                self.assertIn('--hardware ' + hardware, result)
                self.assertIn('--threshold 0.35', result)
                self.assertIn('--no-verifier', result)
                self.assertNotIn('--verifier-dir', result)

    def test_bundled_verifier_in_existing_override_is_turned_off(self):
        self.installed_units()
        path = self.units / 'orion-listener.service.d/99-local.conf'
        self.write(path, '[Service]\nExecStart=\nExecStart=/old/release/voice/.venv/bin/orion-listener '
                   '--threshold 0.47 --verifier-dir /old/release/voice/models/verifier\n')
        result = self.plan()[path]
        self.assertIn('--no-verifier', result)
        self.assertNotIn('--verifier-dir', result)
        self.assertNotIn('/old/release', result)

    def test_removes_retired_environment_and_preserves_other_tuning(self):
        self.env.write_text('# operator tuning\nORION_ASR_THREADS="2"\nORION_TTS_THREADS=4\n'
            'ORION_ASR_CONTEXT="Hey Orion and Ada"\nORION_ASR_MODEL_DIR="/custom/qwen"\n'
            'HF_HOME=/custom/cache\nHF_HUB_OFFLINE=0\nORION_STUDIO_CODEX_BIN=/custom/codex\n'
            'HF_HUB_CACHE=/custom/hub\nHUGGINGFACE_HUB_CACHE=/other/hub\n'
            'ORION_STUDIO_TTS_MODEL=pocket-int8\n'
            'CUSTOM=value')
        old = read_env(self.env.read_text()); result = self.plan()[self.env]; new = read_env(result)
        retired = {'HF_HOME', 'HF_HUB_OFFLINE', 'HF_HUB_CACHE', 'HUGGINGFACE_HUB_CACHE', 'ORION_STUDIO_TTS_MODEL'}
        for key, value in old.items():
            if key not in retired:
                self.assertEqual(new[key], value, key)
        self.assertTrue(retired.isdisjoint(new))
        self.assertEqual(new['ORION_PIPER_MODEL_DIR'], str(self.root / 'models/piper-alba-medium'))
        self.assertIn('# operator tuning\n', result)
        self.assertIn('CUSTOM=value\n', result)
        self.assertEqual(self.env.read_text().splitlines()[-1], 'CUSTOM=value')

    def test_retired_routines_file_is_removed_from_existing_service_commands(self):
        for hardware in ('v1', 'v2'):
            for argument in ('--routines-file /custom/routines.json',
                             '--routines-file "/custom folder/routines.json"',
                             "--routines-file '/custom folder/routines.json'",
                             '--routines-file=/custom/routines.json'):
                for override in (False, True):
                    with self.subTest(hardware=hardware, argument=argument, override=override):
                        (self.release / 'release.json').write_text(json.dumps({'revision': 'new', 'hardware': hardware}))
                        self.installed_units()
                        path = self.units / 'oriond.service'
                        if override:
                            path = self.units / 'oriond.service.d/99-local.conf'
                        original = ('[Service]\nExecStart=\nExecStart=/old/release/runtime/target/release/oriond '
                                    f'--serve {argument} --character-on-start on --rest-after-seconds 1234 '
                                    '--socket /tmp/custom.sock\n')
                        self.write(path, original)
                        plan = self.plan()
                        start = effective_start([(candidate, text) for candidate, text in plan.items()
                                                 if candidate == self.units / 'oriond.service' or candidate.parent == self.units / 'oriond.service.d'])
                        self.assertNotIn('--routines-file', start)
                        self.assertNotIn('routines.json', start)
                        self.assertIn('--character-on-start on', start)
                        self.assertIn('--rest-after-seconds 1234 --socket /tmp/custom.sock', start)
                        self.assertEqual(path.read_text(), original)
                        if override:
                            path.unlink()

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
        self.assertIn('--threshold 0.47', result)
        self.assertIn('--local-processor', result)
        self.assertNotIn('--threshold 0.35', result)
        self.assertIn('hey_orion_reference.rpw', result)

    def test_legacy_managed_listener_migrates_to_active_reference_profile(self):
        self.installed_units()
        path = self.units / 'orion-listener.service.d/40-pi-voice.conf'
        self.write(path, '[Service]\nExecStart=\nExecStart=/old/release/voice/.venv/bin/orion-listener --local-processor --threshold 0.8 --host 0.0.0.0\n')
        result = self.plan()[path]
        self.assertIn('--threshold 0.35', result)
        self.assertIn(str(self.release / 'voice/models/wake/hey_orion_reference.rpw'), result)
        self.assertEqual(result.count('--no-verifier'), 1)

    def test_non_numeric_operator_threshold_is_preserved(self):
        self.installed_units()
        path = self.units / 'orion-listener.service.d/40-pi-voice.conf'
        self.write(path, '[Service]\nExecStart=\nExecStart=/old/release/voice/.venv/bin/orion-listener --threshold ${ORION_THRESHOLD}\n')
        result = self.plan()[path]
        self.assertIn('--threshold ${ORION_THRESHOLD}', result)
        self.assertIn('hey_orion_reference.rpw', result)

    def test_saved_preferences_and_audio_calibration_are_outside_write_set(self):
        for name in ('voice-settings.json', 'microphone.json', 'servo_calibration.json', 'voice.env', 'studio-token'):
            self.write(self.env.parent / name, '{"model":"custom-model"}' if name == 'voice-settings.json' else 'operator settings')
        plan = self.plan()
        for name in ('voice-settings.json', 'microphone.json', 'servo_calibration.json', 'voice.env', 'studio-token'):
            path = self.env.parent / name
            self.assertNotIn(path, plan)
            self.assertEqual(path.read_text(), '{"model":"custom-model"}' if name == 'voice-settings.json' else 'operator settings')
        self.assertNotIn(self.home / '.local/share/orion/SOUL.md', plan)
        self.assertNotIn(self.home / '.local/share/orion/MEMORY.md', plan)

    def test_unsupported_env_and_symlinked_overrides_fail_without_writes(self):
        self.env.write_text('BROKEN="unterminated')
        with self.assertRaises(ValueError): self.plan()
        self.env.unlink()
        path = self.units / 'oriond.service.d/custom.conf'; path.parent.mkdir()
        path.symlink_to(self.env)
        with self.assertRaises(ValueError): self.plan()


class ListenerStartTests(unittest.TestCase):
    release = Path('/new/release')

    def start(self, threshold, model=None):
        start = f'/old/release/voice/.venv/bin/orion-listener --threshold {threshold}'
        if model:
            start += f' --wake-model /old/release/voice/models/wake/{model}'
        return start

    def test_wheel_installs_use_the_asr_wake_check(self):
        result = pi_service_config.listener_start(self.start('0.35'), self.release)
        self.assertIn('--no-verifier', result)
        self.assertNotIn('--verifier-dir', result)

    def test_custom_or_disabled_verifier_is_preserved(self):
        for setting in ('--verifier-dir /custom/verifier', '--verifier-dir=/custom/verifier', '--no-verifier'):
            with self.subTest(setting=setting):
                result = pi_service_config.listener_start(self.start('0.47') + ' ' + setting, self.release)
                self.assertIn(setting, result)
                self.assertNotIn(f'{self.release}/voice/models/verifier', result)

    def test_legacy_managed_threshold_is_rewritten(self):
        for model in (None, pi_service_config.REFERENCE_WAKE_MODEL):
            with self.subTest(model=model):
                result = pi_service_config.listener_start(self.start('0.8', model), self.release)
                self.assertIn('--threshold 0.35', result)
                self.assertNotIn('--threshold 0.8', result)
                self.assertIn(f'--wake-model {self.release}/voice/models/wake/{pi_service_config.ACTIVE_WAKE_MODEL}', result)

    def test_active_managed_threshold_still_receives_wake_model_updates(self):
        for model in (None, pi_service_config.REFERENCE_WAKE_MODEL):
            with self.subTest(model=model), \
                 patch.object(pi_service_config, 'ACTIVE_WAKE_MODEL', 'updated.rpw'):
                result = pi_service_config.listener_start(self.start('0.35', model), self.release)
                self.assertIn('--threshold 0.35', result)
                self.assertIn(f'--wake-model {self.release}/voice/models/wake/updated.rpw', result)

    def test_custom_threshold_is_preserved(self):
        for model in (None, pi_service_config.REFERENCE_WAKE_MODEL):
            with self.subTest(model=model), \
                 patch.object(pi_service_config, 'ACTIVE_WAKE_MODEL', 'updated.rpw'):
                start = self.start('0.6', model)
                result = pi_service_config.listener_start(start, self.release)
                self.assertIn('--threshold 0.6', result)
                self.assertNotIn('--threshold 0.35', result)
                self.assertNotIn('updated.rpw', result)
                if model:
                    self.assertIn(start, result)


class RestRuntimeTests(unittest.TestCase):
    def exercise(self, failures=None, torque_after=False, initial_torque=True, initial_mode='holding', force=False,
                 rest_state='disabled'):
        system = installer.System()
        self.calls = []
        statuses = iter([initial_torque, torque_after])
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
            output = json.dumps({'torque_enabled': next(statuses), 'mode': initial_mode,
                                 'rest': {'state': rest_state}}) if command == '--status' else ''
            return subprocess.CompletedProcess(args, 0, output)

        cmdline = b'oriond\0--serve\0--socket\0/tmp/custom.sock\0'
        with patch.object(system, 'run', side_effect=run), \
             patch.object(installer.Path, 'read_bytes', return_value=cmdline):
            system.rest_runtime(force=force)

    def assert_rests_safely(self):
        """Playback stops before rest, torque goes off only after rest, then is re-checked."""
        calls = self.calls
        for stop in ('--stop-scene', '--stop-speech', '--stop'):
            self.assertLess(calls.index(stop), calls.index('--goto'))
        self.assertLess(calls.index('--goto'), calls.index('--disable'))
        self.assertEqual(calls.count('--disable'), 1)
        self.assertEqual(calls[-1], '--status')

    def test_a_torque_off_robot_in_another_pose_is_powered_and_rested_before_switching(self):
        for mode in ('observe', 'configured'):
            self.exercise(initial_torque=False, initial_mode=mode, force=True)
            if mode == 'observe': self.assertIn('--configure', self.calls)
            self.assertLess(self.calls.index('--enable'), self.calls.index('--goto'))
            self.assertLess(self.calls.index('--goto'), self.calls.index('--disable'))

    def test_forced_rest_remeasures_even_when_an_old_rest_state_is_still_latched(self):
        self.exercise(initial_torque=False, initial_mode='configured', force=True, rest_state='resting')
        self.assertIn('--enable', self.calls)
        self.assertIn('--goto', self.calls)

    def test_active_and_already_inactive_playback_both_reach_confirmed_rest(self):
        for scene in (False, True):
            for speech in (False, True):
                with self.subTest(scene_inactive=scene, speech_inactive=speech):
                    failures = {}
                    if scene: failures['--stop-scene'] = (3, '{"ok":false,"error":"No scene is active."}')
                    if speech: failures['--stop-speech'] = (3, '{"ok":false,"error":"No speech run is active."}')
                    self.exercise(failures)
                    self.assert_rests_safely()

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
        self.assert_rests_safely()
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


class SmokeRuntimeTests(Fixture):
    def exercise(self, failed_command=None):
        system = installer.System()
        client = ['/proc/42/exe', '--socket', '/tmp/custom.sock']
        calls = []
        run_id = 0
        initial = True
        def command(_, value):
            nonlocal run_id
            if value == 'character status':
                return dict(ok=True, character={}, rest=dict(state='resting', light_on=False))
            self.assertEqual(value, 'character rest')
            run_id += 1
            calls.append(('character rest', run_id))
            return {'ok': True, 'run_id': run_id}
        def run(*args, **kwargs):
            nonlocal initial
            self.assertEqual(list(args[:3]), client)
            calls.append(tuple(args[3:]))
            if args[3:5] == failed_command:
                raise subprocess.CalledProcessError(6, args, output='smoke failed')
            if args[3] == '--status':
                if initial:
                    initial = False
                    value = dict(build_revision='new', mode='moving', torque_enabled=True)
                else:
                    value = dict(build_revision='new', mode='configured', torque_enabled=False,
                        last_motion=dict(run_id=run_id, state='completed'))
                return subprocess.CompletedProcess(args, 0, json.dumps(value))
            return subprocess.CompletedProcess(args, 0)
        with patch.object(system, 'runtime_client', return_value=client), \
             patch.object(system, 'run', side_effect=run), \
             patch.object(system, 'daemon_command', side_effect=command):
            if failed_command:
                with self.assertRaises(subprocess.CalledProcessError): system.smoke_runtime(self.release)
            else:
                system.smoke_runtime(self.release)
        return calls

    def test_smoke_waits_for_each_scene_and_returns_to_the_wakeable_rest_lifecycle(self):
        calls = self.exercise()
        self.assertLess(calls.index(('character rest', 1)), calls.index(('--enable',)))
        scenes = [c for c in calls if c[0] == '--run-scene']
        self.assertEqual(scenes, [('--run-scene', scene, '--wait') for scene in
            ('deployment_smoke', 'acknowledge_left', 'acknowledge_right', 'return_home')])
        self.assertIn(('--goto', 'zero_reference', '--duration', '3.0', '--wait'), calls)
        self.assertGreater(calls.index(('character rest', 2)), calls.index(scenes[-1]))
        self.assertNotIn(('--disable',), calls)

    def test_failed_pose_or_scene_still_attempts_measured_rest_and_fails_deployment(self):
        for command in (('--goto', 'zero_reference'), ('--run-scene', 'deployment_smoke'),
                        ('--run-scene', 'acknowledge_left'), ('--run-scene', 'acknowledge_right'),
                        ('--run-scene', 'return_home')):
            with self.subTest(command=command):
                calls = self.exercise(command)
                self.assertEqual([c for c in calls if c[0] == 'character rest'],
                                 [('character rest', 1), ('character rest', 2)])

    def test_rest_requires_the_matching_completed_run_torque_off_and_dark_lights(self):
        for fault in (None, 'wrong-run', 'cancelled', 'timed_out', 'torque', 'light', 'state', 'fault', 'missing-rest'):
            with self.subTest(fault=fault):
                # The real runtime exposes these through two separate commands.
                status = dict(last_motion=dict(run_id=7, state='completed'), torque_enabled=False)
                lifecycle = dict(ok=True, character={}, rest=dict(state='resting', light_on=False))
                if fault == 'wrong-run': status['last_motion']['run_id'] = 6
                if fault in ('cancelled', 'timed_out'): status['last_motion']['state'] = fault
                if fault == 'torque': status['torque_enabled'] = True
                if fault == 'light': lifecycle['rest']['light_on'] = True
                if fault == 'state': lifecycle['rest']['state'] = 'going_to_rest'
                if fault == 'fault': lifecycle['rest']['state'] = 'fault'
                if fault == 'missing-rest': lifecycle.pop('rest')
                def command(client, value):
                    self.assertIn(value, ('character rest', 'character status'))
                    return dict(ok=True, run_id=7) if value == 'character rest' else lifecycle
                system = installer.System()
                with patch.object(system, 'daemon_command', side_effect=command) as daemon, \
                     patch.object(system, 'run', return_value=subprocess.CompletedProcess([], 0, json.dumps(status))) as run, \
                     patch.object(installer.time, 'monotonic', side_effect=[0, 0, 31]), \
                     patch.object(installer.time, 'sleep'):
                    if fault:
                        with self.assertRaises(RuntimeError): system.settle_runtime(['/proc/42/exe', '--socket', '/tmp/custom.sock'])
                    else:
                        system.settle_runtime(['/proc/42/exe', '--socket', '/tmp/custom.sock'])
                    self.assertEqual([call.args[1] for call in daemon.call_args_list],
                                     ['character rest', 'character status'])
                    self.assertFalse(any('--disable' in call.args for call in run.call_args_list))

    def test_rest_polls_movement_and_lifecycle_until_both_confirm_completion(self):
        system = installer.System()
        def command(client, value):
            if value == 'character rest':
                return dict(ok=True, run_id=7)
            return dict(ok=True, rest=dict(state='resting', light_on=False))
        # The rest lifecycle can finish between the two reads. A completed
        # lifecycle alone cannot substitute for measured movement/torque status.
        statuses = [dict(motion=dict(run_id=7, state='settling'), last_motion=None, torque_enabled=True),
                    dict(motion=None, last_motion=dict(run_id=7, state='completed'), torque_enabled=False)]
        with patch.object(system, 'daemon_command', side_effect=command), \
             patch.object(system, 'run', side_effect=[subprocess.CompletedProcess([], 0, json.dumps(s)) for s in statuses]) as run, \
             patch.object(installer.time, 'sleep'):
            system.settle_runtime(['runtime', '--socket', 'socket'])
            self.assertEqual(run.call_count, 2)

    def test_initial_rest_failure_does_not_claim_smoke_started_or_retry_rest(self):
        system = installer.System()
        with patch.object(system, 'wait_runtime_client', return_value=(['runtime', '--socket', 'socket'], {})), \
             patch.object(system, 'settle_runtime', side_effect=RuntimeError('initial rest failed')) as rest, \
             patch.object(system, 'run') as run, patch('builtins.print') as output:
            with self.assertRaisesRegex(RuntimeError, 'initial rest failed'):
                system.smoke_runtime(self.release)
            rest.assert_called_once()
            run.assert_not_called()
            self.assertFalse(any('Running the physical' in str(call) for call in output.call_args_list))

    def test_smoke_failure_is_preserved_when_final_rest_also_fails(self):
        system = installer.System()
        failed = subprocess.CalledProcessError(6, ['runtime', '--run-scene', 'deployment_smoke'])
        with patch.object(system, 'wait_runtime_client', return_value=(['runtime', '--socket', 'socket'], {})), \
             patch.object(system, 'settle_runtime', side_effect=[None, RuntimeError('rest failed')]), \
             patch.object(system, 'stop_playback'), \
             patch.object(system, 'run', side_effect=[subprocess.CompletedProcess([], 0, '{"mode":"holding"}'),
                                                     subprocess.CompletedProcess([], 0), failed]):
            with self.assertRaisesRegex(RuntimeError, 'Physical smoke failed:.*deployment_smoke.*final rest also failed: rest failed'):
                system.smoke_runtime(self.release)

    def test_rest_timeout_reports_the_run_and_observed_runtime_states(self):
        system = installer.System()
        status = dict(last_motion=dict(run_id=7, state='completed'), torque_enabled=False)
        def command(client, value):
            return dict(ok=True, run_id=7) if value == 'character rest' else dict(ok=True, rest=dict(state='going_to_rest', light_on=True))
        with patch.object(system, 'daemon_command', side_effect=command), \
             patch.object(system, 'run', return_value=subprocess.CompletedProcess([], 0, json.dumps(status))), \
             patch.object(installer.time, 'monotonic', side_effect=[0, 0, 31]), patch.object(installer.time, 'sleep'):
            with self.assertRaisesRegex(RuntimeError, r"rest run 7.*completed.*torque_enabled=False.*going_to_rest.*light_on.*True"):
                system.settle_runtime(['runtime', '--socket', 'socket'])


class RuntimeRestartTests(Fixture):
    """A crashed oriond is restarted by systemd under a new PID."""

    def client_for(self, link):
        system = installer.System()
        cmdline = b'oriond\0--serve\0--socket\0/tmp/custom.sock\0'
        with patch.object(system, 'run', return_value=subprocess.CompletedProcess([], 0, '42\n')), \
             patch.object(installer.Path, 'read_bytes', return_value=cmdline), \
             patch.object(installer.os, 'readlink', side_effect=link):
            return system.runtime_client()

    def test_client_uses_the_release_binary_not_the_pid_link(self):
        binary = '/home/pi/.local/share/orion/voice-stack/releases/r/runtime/target/release/oriond'
        self.assertEqual(self.client_for(lambda path: binary), [binary, '--socket', '/tmp/custom.sock'])

    def test_client_falls_back_to_the_pid_link_when_the_binary_cannot_be_resolved(self):
        for link in (OSError('no /proc'), lambda path: '/old/oriond (deleted)', lambda path: 'relative/oriond'):
            with self.subTest(link=link):
                self.assertEqual(self.client_for(link), ['/proc/42/exe', '--socket', '/tmp/custom.sock'])

    def test_failed_smoke_reconnects_before_the_final_rest(self):
        system = installer.System()
        old, new = ['old-runtime', '--socket', 's'], ['new-runtime', '--socket', 's']
        failed = subprocess.CalledProcessError(1, old + ['--goto', 'zero_reference'])
        with patch.object(system, 'wait_runtime_client', side_effect=[(old, {}), (new, {})]) as wait, \
             patch.object(system, 'settle_runtime') as rest, \
             patch.object(system, 'stop_playback'), \
             patch.object(system, 'run', side_effect=[subprocess.CompletedProcess([], 0, '{"mode":"holding"}'), failed]):
            with self.assertRaises(subprocess.CalledProcessError):
                system.smoke_runtime(self.release)
        self.assertEqual([call.args[0] for call in rest.call_args_list], [old, new])
        self.assertEqual([call.args[0] for call in wait.call_args_list], ['new', 'new'])

    def test_successful_smoke_keeps_its_client(self):
        system = installer.System()
        client = ['runtime', '--socket', 's']
        with patch.object(system, 'wait_runtime_client', return_value=(client, {})) as wait, \
             patch.object(system, 'settle_runtime') as rest, \
             patch.object(system, 'stop_playback'), \
             patch.object(system, 'run', return_value=subprocess.CompletedProcess([], 0, '{"mode":"holding"}')):
            system.smoke_runtime(self.release)
        wait.assert_called_once()
        self.assertEqual(rest.call_count, 2)

    def test_v2_smoke_uses_the_measured_home_pose(self):
        (self.release / 'release.json').write_text('{"revision":"new","hardware":"v2"}')
        calls = SmokeRuntimeTests.exercise(self)
        self.assertIn(('--goto', 'home', '--duration', '3.0', '--wait'), calls)
        self.assertFalse(any('zero_reference' in call for call in calls))

    def systemctl(self, *states):
        outputs = iter(states)
        def run(args, **kwargs):
            return subprocess.CompletedProcess(args, 0,
                f'LoadState=loaded\nActiveState={next(outputs)}\nUnitFileState=enabled\n')
        return run

    def test_state_waits_for_a_restarting_unit_to_settle(self):
        system = installer.System()
        with patch.object(installer.subprocess, 'run', side_effect=self.systemctl('activating', 'activating', 'active')), \
             patch.object(installer.time, 'monotonic', side_effect=[0, 1, 2]), \
             patch.object(installer.time, 'sleep') as sleep:
            self.assertEqual(system.state('oriond'), {'active': True, 'enabled': 'enabled'})
        self.assertEqual(sleep.call_count, 2)

    def test_state_reports_a_unit_that_never_settles_with_its_journal(self):
        system = installer.System()
        with patch.object(installer.subprocess, 'run', side_effect=self.systemctl('activating', 'deactivating')), \
             patch.object(installer.time, 'monotonic', side_effect=[0, 10, 31]), \
             patch.object(installer.time, 'sleep'), \
             patch.object(installer, 'journal_tail', return_value='\nRecent oriond log:\nbus timed out'):
            with self.assertRaisesRegex(RuntimeError, r'(?s)oriond stayed deactivating for 30s.*crash-looping.*bus timed out'):
                system.state('oriond')


class SystemWriteTests(Fixture):
    def test_user_files_get_user_owned_parent_directories_before_sudo(self):
        target = self.home / '.local/share/orion/studio-service/installed'
        system = installer.System()
        calls = []
        def run(*args, **kwargs):
            # The parent must already exist (owned by us) when sudo runs.
            calls.append((args[:2], target.parent.is_dir()))
            return subprocess.CompletedProcess(args, 0)
        with patch.object(system, 'run', side_effect=run):
            system.write(target, b'release\n', 0o600)
        self.assertTrue(calls)
        self.assertTrue(all(parent_exists for _, parent_exists in calls))

    def test_system_files_leave_directory_creation_to_sudo(self):
        system = installer.System()
        with patch.object(system, 'run', return_value=subprocess.CompletedProcess([], 0)), \
             patch.object(installer.Path, 'mkdir') as mkdir:
            system.write(Path('/etc/systemd/system/oriond.service'), b'unit', 0o644)
        mkdir.assert_not_called()


class DeployHardeningTests(Fixture):
    def calibration(self, **override):
        joints = {name: {'servo_id': info['servo_id'], 'direction': 1,
                         'min_radians': -0.5, 'max_radians': 0.5}
                  for name, info in installer.profile('v2')['joints'].items()}
        for name, values in override.items():
            joints[name].update(values)
        path = self.base / 'calibration.json'
        path.write_text(json.dumps({'joints': joints}))
        return path

    def test_calibration_crossing_the_encoder_seam_or_wrong_ids_is_refused_before_switching(self):
        installer.check_calibration(self.calibration(), 'v2')
        with self.assertRaisesRegex(RuntimeError, 'profile crosses 0/4095'):
            installer.check_calibration(self.calibration(base_yaw_joint={'max_radians': 4.0}), 'v2')
        with self.assertRaisesRegex(RuntimeError, 'Invalid V2 joint mapping: head_roll_joint'):
            installer.check_calibration(self.calibration(head_roll_joint={'servo_id': 4}), 'v2')

    def test_hand_started_runtime_is_found_and_systemds_own_is_ignored(self):
        def run(args, **kwargs):
            output = '100\n' if args[0] == 'systemctl' else '100\n4321\n'
            return subprocess.CompletedProcess(args, 0, output)
        with patch.object(installer.subprocess, 'run', side_effect=run):
            self.assertEqual(installer.System().foreign_runtimes(), ['4321'])

    def test_success_keeps_only_the_active_and_rollback_releases(self):
        releases = self.base / 'releases'
        names = ['aaaaaaaaaaaa-00000001', 'bbbbbbbbbbbb-00000002', 'cccccccccccc-00000003']
        for name in names:
            (releases / name).mkdir(parents=True)
        (releases / 'keep-me').mkdir()
        with patch('builtins.print'):
            installer.prune_releases(releases / names[2], str(releases / names[1]))
        self.assertEqual(sorted(p.name for p in releases.iterdir()), sorted(names[1:] + ['keep-me']))

    def test_rest_reconnects_when_the_runtime_restarted(self):
        system = installer.System()
        client = ['runtime', '--socket', 's']
        status = subprocess.CompletedProcess([], 0, json.dumps({'torque_enabled': False}))
        with patch.object(system, 'wait_runtime_client', return_value=(client, {'torque_enabled': False})) as wait, \
             patch.object(system, 'runtime_client', side_effect=AssertionError('stale client')), \
             patch.object(system, 'run', return_value=status):
            system.rest_runtime()
        wait.assert_called_once()


class FakeSystem:
    """Model unit existence separately from enablement to catch dangling wants links."""
    def __init__(self, units, existing=True):
        self.units = units
        self.states = {name: {'active': existing, 'enabled': 'enabled' if existing else 'not-found'} for name in SERVICES}
        self.states['orion-vision'] = {'active': False, 'enabled': 'not-found'}
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
    def rest_runtime(self, force=False):
        self.calls.append(('rest',))
        if self.failure == 'rest': raise RuntimeError('Unsafe rest')
    def smoke_runtime(self, release):
        self.calls.append(('smoke',))
        if self.failure == 'smoke':
            self.failure = None; raise RuntimeError('Injected smoke failure')
    def ready(self, release, home):
        self.calls.append(('ready',))
        if self.failure in ('ready', 'interrupt'):
            failure = self.failure; self.failure = None
            if failure == 'interrupt': raise KeyboardInterrupt()
            raise RuntimeError('Injected readiness failure')


class TransactionTests(Fixture):
    def test_manual_rollback_restores_packages_after_stopping_services(self):
        self.activate()
        called = []
        def restore_packages(folder, data):
            self.assertFalse(any(state['active'] for state in self.system.states.values()))
            called.append(folder)
        installer.rollback(self.root, self.system, before_restore=restore_packages)
        self.assertEqual(len(called), 1)
        self.assert_restored()

    def test_failed_activation_restores_packages_before_old_services(self):
        self.system.failure = 'ready'
        called = []
        def restore_packages(folder, data):
            self.assertFalse(any(state['active'] for state in self.system.states.values()))
            called.append(folder)
        with self.assertRaisesRegex(RuntimeError, 'readiness'):
            installer.activate(self.root, self.release, self.home, self.plan(), self.system,
                               before_restore=restore_packages)
        self.assertEqual(len(called), 1)
        self.assert_restored()

    def test_camera_activation_failure_removes_new_unit_and_restores_previous_states(self):
        (self.release / 'release.json').write_text('{"revision":"new","hardware":"v2"}')
        self.manifest.write_text('{"release":"/old/release","hardware":"v2"}')
        original = self.manifest.read_bytes()
        self.system.failure = 'ready'
        with self.assertRaisesRegex(RuntimeError, 'readiness'):
            self.activate()
        self.assertFalse((self.units / 'orion-vision.service').exists())
        self.assertEqual(self.system.states, self.original_states)
        self.assertEqual(self.manifest.read_bytes(), original)
        self.assertIn(('start', 'orion-vision'), self.system.calls)

    def test_non_camera_release_disables_existing_camera_without_deleting_preference(self):
        self.write(self.units / 'orion-vision.service', '[Service]\nExecStart=/old/vision\n')
        self.system.states['orion-vision'] = {'active': True, 'enabled': 'enabled'}
        privacy = self.env.parent / 'camera.json'
        privacy.write_text('{"camera_enabled":false}')
        self.activate()
        self.assertEqual(self.system.states['orion-vision'], {'active': False, 'enabled': 'disabled'})
        self.assertEqual(privacy.read_text(), '{"camera_enabled":false}')
        installer.rollback(self.root, self.system)
        self.assertEqual(self.system.states['orion-vision'], {'active': True, 'enabled': 'enabled'})

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
        for failure in ('write', 'start', 'smoke', 'ready', 'interrupt'):
            with self.subTest(failure=failure):
                self.system.failure = failure
                with self.assertRaises((RuntimeError, KeyboardInterrupt)): self.activate()
                self.assert_restored()

    def test_smoke_runs_after_switching_and_before_companions_start(self):
        self.activate()
        calls = self.system.calls
        smoke = calls.index(('smoke',))
        self.assertLess(calls.index(('rest',)), calls.index(('stop', 'oriond')))
        self.assertLess(calls.index(('stop', 'oriond')), calls.index(('write', str(self.env))))
        self.assertLess(calls.index(('start', 'oriond')), smoke)
        for name in SERVICES[1:]:
            self.assertGreater(calls.index(('start', name)), smoke)
        self.assertGreater(calls.index(('ready',)), smoke)

    def test_an_inactive_installed_runtime_is_started_and_rested_before_switch(self):
        self.system.states['oriond']['active'] = False
        self.activate()
        calls = self.system.calls
        self.assertLess(calls.index(('start', 'oriond')), calls.index(('rest',)))
        self.assertLess(calls.index(('rest',)), calls.index(('write', str(self.env))))

    def test_unsafe_rest_does_not_stop_runtime_or_change_config(self):
        self.system.failure = 'rest'
        with self.assertRaisesRegex(RuntimeError, 'Unsafe rest'): self.activate()
        self.assertNotIn(('stop', 'oriond'), self.system.calls)
        self.assert_restored()

    def test_home_container_and_settings_are_outside_install_update_and_rollback(self):
        config = self.home / '.local/share/orion/home-assistant/configuration.yaml'
        token = self.home / '.config/orion/home-assistant.json'
        metadata = self.home / '.local/share/orion/home-assistant-install.json'
        for path in (config, token, metadata):
            self.write(path, 'independent home state')
        self.activate()
        installer.rollback(self.root, self.system)
        for path in (config, token, metadata):
            self.assertEqual(path.read_text(), 'independent home state')
        for call in self.system.calls:
            self.assertFalse(any(value in ('docker', 'orion-home-assistant', 'orion-home-assistant-install') for value in call))

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
        (self.release / 'scripts').mkdir()
        for name in ('home_assistant_install.py', 'release-home-assistant.json'):
            shutil.copy2(SCRIPTS / name, self.release / 'scripts' / name)
        shutil.copytree(SCRIPTS.parent / 'hardware/profiles', self.release / 'hardware/profiles')
        (self.release / 'release.json').write_text('{"revision":"failed"}')
        self.system.failure = 'ready'
        with self.assertRaises(RuntimeError): self.activate()
        self.assertEqual(self.manifest.read_bytes(), previous_manifest)
        self.assertTrue(first.exists())

    def test_failed_recovery_keeps_journal_and_blocks_new_update(self):
        def fail_readiness(*args):
            self.system.fail_restore = True
            raise RuntimeError('Injected readiness failure')
        with patch.object(self.system, 'ready', side_effect=fail_readiness):
            with self.assertRaisesRegex(RuntimeError, 'Recovery could not finish') as failure: self.activate()
        self.assertIn('Activation failed: RuntimeError: Injected readiness failure', str(failure.exception))
        self.assertIn('Recovery failed: RuntimeError: Cannot restore', str(failure.exception))
        pending = self.root / 'pending-installation.json'
        self.assertTrue(pending.exists())
        transaction = Path(json.loads(pending.read_text())['transaction'])
        state = json.loads((transaction / 'state.json').read_text())
        self.assertEqual(state['activation_error'], 'RuntimeError: Injected readiness failure')
        self.assertEqual(state['recovery_error'], 'RuntimeError: Cannot restore')
        with self.assertRaisesRegex(RuntimeError, 'interrupted deployment'): self.activate()
        self.system.fail_restore = False
        installer.rollback(self.root, self.system)
        self.assert_restored()

    def test_failed_smoke_and_rest_preserve_daemon_response_without_stopping_runtime(self):
        # The primary configuration rejection must survive a subsequent failure
        # to contact the hardware owner during recovery.
        self.system.calls.clear()
        activation_error = RuntimeError('Runtime rejected character rest: servo configuration failed')
        recovery_error = subprocess.CalledProcessError(3, ['runtime', '--status'],
            output=b'{"ok":false,"error":"driver disconnected"}', stderr=b'socket unavailable')
        with patch.object(self.system, 'smoke_runtime', side_effect=activation_error), \
             patch.object(self.system, 'rest_runtime', side_effect=[None, recovery_error]):
            with self.assertRaises(RuntimeError) as failure:
                self.activate()
        self.assertIs(failure.exception.__cause__, activation_error)
        self.assertIn('servo configuration failed', str(failure.exception))
        self.assertIn('driver disconnected', str(failure.exception))
        self.assertIn('socket unavailable', str(failure.exception))
        self.assertEqual(self.system.calls.count(('stop', 'oriond')), 1)
        self.assertTrue(self.system.states['oriond']['active'])
        pending = json.loads((self.root / 'pending-installation.json').read_text())
        state = json.loads((Path(pending['transaction']) / 'state.json').read_text())
        self.assertEqual(state['status'], 'rollback_failed')
        self.assertIn('driver disconnected', state['recovery_error'])

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
        cases = [(fault, None) for fault in (None, 'old-service', 'old-runtime', 'wrong-gateway', 'no-asr', 'no-tts',
                      'wrong-tts-provider', 'wrong-tts-model', 'no-agent', 'wrong-agent-model',
                      'wrong-wake-model', 'wrong-wake-threshold', 'no-verifier', 'not-resting', 'lights-on', 'torque-on')]
        cases += [(None, '0.35'), (None, '0.8'), (None, '0.6'), ('wrong-wake-threshold', '0.6')]
        cases = [(fault, threshold, 'v1') for fault, threshold in cases]
        cases += [(None, None, 'v2'), ('gaze-unavailable', None, 'v2'), ('vision-detection', None, 'v2')]
        for fault, listener_threshold, hardware in cases:
            with self.subTest(fault=fault, listener_threshold=listener_threshold):
                (self.release / 'release.json').write_text(json.dumps({'revision': 'new', 'hardware': hardware}))
                status = dict(coordinator_running=True, error=None, project_root=str(self.release), revision='new', pid=42)
                event = dict(type='ready', asr=dict(provider='qwen3-asr'),
                             tts=dict(provider='piper-tts', model='piper-alba-medium'),
                             agent=dict(provider='codex', model='gpt-6-luna', effort='medium'),
                             wake=dict(provider='rustpotter', model='hey_orion_reference.rpw',
                                       threshold=float(listener_threshold or '0.35'),
                                       verifier=dict(provider='openwakeword', active=True)))
                revision, gateway_pid = 'new', 42
                if fault == 'old-service': status['project_root'] = '/old/release'
                if fault == 'old-runtime': revision = 'old'
                if fault == 'wrong-gateway': gateway_pid = 77
                for key in ('asr', 'tts', 'agent'):
                    if fault == 'no-'+key: event[key] = {}
                if fault == 'wrong-tts-provider': event['tts']['provider'] = 'pocket-tts'
                if fault == 'wrong-tts-model': event['tts']['model'] = 'pocket-int8'
                if fault == 'wrong-agent-model': event['agent']['model'] = 'gpt-5.6-sol'
                if fault == 'wrong-wake-model': event['wake']['model'] = 'hey_orion_trained_080.rpw'
                if fault == 'wrong-wake-threshold': event['wake']['threshold'] = 0.8
                if fault == 'no-verifier': event['wake']['verifier'] = None
                system = installer.System()
                self.write(self.home / '.config/orion/custom-token', 'fixture-token')
                cmdline = '\0'.join(['python3', 'gateway.py', '--port', '7555', '--socket', '/tmp/custom.sock', '--token-file', str(self.home / '.config/orion/custom-token'), '']).encode()
                listener_args = ['orion-listener']
                if listener_threshold is not None:
                    listener_args += ['--threshold', listener_threshold]
                listener_cmdline = '\0'.join(listener_args + ['']).encode()
                def run(*args, **kwargs):
                    if args[0] == 'systemctl': return subprocess.CompletedProcess(args, 0, '100\n')
                    self.assertIn('/tmp/custom.sock', args)
                    return subprocess.CompletedProcess(args, 0, json.dumps(dict(build_revision=revision,
                        hardware=hardware, torque_enabled=fault == 'torque-on')))
                def command(client, value):
                    self.assertEqual(str(client[0]), str(self.release / 'runtime/target/release/oriond'))
                    self.assertEqual(client[1:], ['--socket', '/tmp/custom.sock'])
                    if value == 'gaze status':
                        return dict(ok=True, gaze=dict(state='unavailable' if fault == 'gaze-unavailable' else 'stopped',
                                                       error='Missing camera frames' if fault == 'gaze-unavailable' else None))
                    self.assertEqual(value, 'character status')
                    return dict(ok=True, rest=dict(state='going_to_rest' if fault == 'not-resting' else 'resting',
                                                  light_on=fault == 'lights-on'))
                def request(req, **kwargs):
                    self.assertEqual(req.full_url, 'http://127.0.0.1:7555/api/v2/voice/request')
                    self.assertEqual(req.headers['Authorization'], 'Bearer fixture-token')
                    return io.BytesIO(json.dumps(dict(pid=gateway_pid)).encode())
                with patch.object(system, 'state', return_value=dict(active=True)), \
                     patch.object(system, 'run', side_effect=run), \
                     patch.object(system, 'daemon_command', side_effect=command), \
                     patch.object(installer.Path, 'read_bytes', side_effect=[cmdline, listener_cmdline]), \
                     patch.object(installer, 'control', side_effect=[status, dict(events=[event])]), \
                     patch.object(installer.time, 'monotonic', side_effect=[0, 0, 2]), \
                     patch.object(installer.time, 'sleep'), \
                     patch.object(installer, 'check_vision_health', side_effect=RuntimeError('Vision detection error: model missing')
                                  if fault == 'vision-detection' else None), \
                     patch.object(installer.urllib.request, 'urlopen', side_effect=request):
                    if fault:
                        with self.assertRaisesRegex(RuntimeError, 'did not become ready'):
                            system.ready(self.release, self.home, timeout=1)
                    else:
                        system.ready(self.release, self.home, timeout=1)

    def test_readiness_migrates_old_voice_choices(self):
        self.assertEqual(installer.expected_tts(self.home), ('piper-tts', 'piper-alba-medium'))
        self.env.write_text('ORION_STUDIO_TTS_MODEL=pocket-fp32\n')
        self.assertEqual(installer.expected_tts(self.home), ('piper-tts', 'piper-alba-medium'))
        self.write(self.env.parent / 'voice-settings.json', '{"ttsModel":"pocket-int8","ttsVoice":"jane","ttsPath":"/old/model"}')
        self.assertEqual(installer.expected_tts(self.home), ('piper-tts', 'piper-alba-medium'))
        self.write(self.env.parent / 'voice-settings.json', '{"ttsModel":"unsupported"}')
        with self.assertRaisesRegex(RuntimeError, 'Unsupported Pi speech model'):
            installer.expected_tts(self.home)

    def test_preflight_uses_saved_env_without_mutation(self):
        self.env.write_text('ORION_STUDIO_CODEX_BIN=/custom/codex\nORION_TTS_THREADS=2\n')
        for name in ('runtime/target/release/oriond', 'runtime/target/release/orion-trajectory',
                     'orion-service/target/release/orion-service', 'speech/.venv/bin/python', 'voice/.venv/bin/orion-listener'):
            self.write(self.release / name, 'fixture')
        self.write(self.env.parent / 'servo_calibration.json', '{"joints": {}}')
        self.write(self.env.parent / 'studio-token', 'fixture')
        system = installer.System()
        before = self.env.read_bytes()
        with patch.object(system, 'run') as run:
            installer.preflight(self.release, self.home, self.plan(), system)
        self.assertEqual(run.call_args_list[0].args, ('/custom/codex', 'login', 'status'))
        self.assertEqual(run.call_args.kwargs['env']['ORION_TTS_THREADS'], '2')
        self.assertEqual(run.call_args.args, (self.release / 'orion-service/target/release/orion-service', 'check'))
        self.assertEqual(self.env.read_bytes(), before)

    def test_preflight_reports_the_selected_missing_calibration_before_external_checks(self):
        for hardware in ('v1', 'v2'):
            with self.subTest(hardware=hardware):
                (self.release / 'release.json').write_text(json.dumps({'hardware': hardware}))
                for name in ('runtime/target/release/oriond', 'runtime/target/release/orion-trajectory',
                             'orion-service/target/release/orion-service', 'speech/.venv/bin/python',
                             'voice/.venv/bin/orion-listener'):
                    self.write(self.release / name, 'fixture')
                token = self.env.parent / 'studio-token'
                self.write(token, 'existing pairing token')
                system = installer.System()
                with patch.object(system, 'run') as run:
                    with self.assertRaises(RuntimeError) as error:
                        installer.preflight(self.release, self.home, self.plan(), system)
                expected = installer.calibration_path(self.home, hardware, project=self.release)
                if hardware == 'v1':
                    self.assertEqual(str(error.exception), f'Missing {hardware} servo calibration: {expected}')
                else:
                    self.assertIn('vision/.venv/bin/orion-vision', str(error.exception))
                run.assert_not_called()
                self.assertEqual(token.read_text(), 'existing pairing token')

    def test_preflight_reports_missing_token_without_misreporting_saved_v2_calibration(self):
        (self.release / 'release.json').write_text('{"hardware":"v2"}')
        for name in ('runtime/target/release/oriond', 'runtime/target/release/orion-trajectory',
                     'orion-service/target/release/orion-service', 'speech/.venv/bin/python',
                     'voice/.venv/bin/orion-listener'):
            self.write(self.release / name, 'fixture')
        calibration = installer.calibration_path(self.home, 'v2', project=self.release)
        original_profile = calibration.read_text()
        token = self.env.parent / 'studio-token'
        system = installer.System()
        with patch.object(system, 'run') as run:
            with self.assertRaises(RuntimeError) as error:
                installer.preflight(self.release, self.home, self.plan(), system)
        self.assertIn(f'Missing Studio pairing token: {token}', str(error.exception))
        self.assertIn(f'create-token --token-file {token}', str(error.exception))
        self.assertNotIn('Missing v2 servo calibration', str(error.exception))
        run.assert_not_called()
        self.assertEqual(calibration.read_text(), original_profile)
        self.assertFalse(token.exists())


class CommissioningTests(Fixture):
    def test_rest_frame_update_resumes_units_after_successful_check(self):
        (self.release / 'release.json').write_text('{"hardware":"v2","joint_frame":"mechanical_rest"}')
        old = self.root / 'releases/old'; old.mkdir()
        (old / 'release.json').write_text('{"hardware":"v2","joint_frame":"mechanical_rest"}')
        (self.root / 'installation.json').write_text(json.dumps({'version': 2, 'release': str(old), 'hardware': 'v2'}))
        system = FakeSystem(self.units)
        with patch.object(installer.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)):
            folder = installer.activate(self.root, self.release, self.home, self.plan(), system, commissioning=True)
        self.assertTrue(all(state['active'] for state in system.states.values()))
        self.assertTrue(json.loads((folder / 'state.json').read_text())['services_started'])

    def test_old_v2_frame_stays_stopped_even_if_raw_readings_pass_new_limits(self):
        (self.release / 'release.json').write_text('{"hardware":"v2","joint_frame":"mechanical_rest"}')
        old = self.root / 'releases/old'; old.mkdir()
        (old / 'release.json').write_text('{"hardware":"v2"}')
        (self.root / 'installation.json').write_text(json.dumps({'version': 2, 'release': str(old), 'hardware': 'v2'}))
        system = FakeSystem(self.units)
        with patch.object(installer.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)):
            installer.activate(self.root, self.release, self.home, self.plan(), system, commissioning=True)
        self.assertTrue(all(not state['active'] for state in system.states.values()))
        installed = json.loads((self.root / 'installation.json').read_text())
        state = json.loads((Path(installed['rollback']) / 'state.json').read_text())
        self.assertTrue(state['crosses_rest_frame'])

    def test_first_install_stays_stopped_even_if_direct_check_passes(self):
        (self.release / 'release.json').write_text('{"hardware":"v2","joint_frame":"mechanical_rest"}')
        system = FakeSystem(self.units, existing=False)
        with patch.object(installer.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)):
            installer.activate(self.root, self.release, self.home, self.plan(), system, commissioning=True)
        self.assertTrue((self.root / 'installation.json').is_file())
        self.assertTrue(all(not state['active'] for state in system.states.values()))
        self.assertTrue(all(state['enabled'] == 'enabled' for state in system.states.values()))
        installed = json.loads((self.root / 'installation.json').read_text())
        self.assertNotIn('services_started', json.loads((Path(installed['rollback']) / 'state.json').read_text()))

    def test_failed_direct_check_installs_units_but_never_starts_motion(self):
        (self.release / 'release.json').write_text('{"hardware":"v2","joint_frame":"mechanical_rest"}')
        system = FakeSystem(self.units, existing=False)
        with patch.object(installer.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1)) as check:
            installer.activate(self.root, self.release, self.home, self.plan(), system, commissioning=True)
        self.assertIn('--check', check.call_args.args[0])
        self.assertNotIn(('start', 'oriond'), system.calls)
        self.assertTrue((self.root / 'installation.json').is_file())
        self.assertTrue(all(not state['active'] for state in system.states.values()))
        self.assertTrue(all(state['enabled'] == 'enabled' for state in system.states.values()))

    def test_manual_rollback_cannot_restart_the_old_coordinate_frame(self):
        folder = self.root / 'transactions' / ('a' * 32); folder.mkdir(parents=True)
        (folder / 'state.json').write_text('{"status":"committed","crosses_rest_frame":true}')
        (self.root / 'installation.json').write_text(json.dumps({'rollback': str(folder)}))
        system = FakeSystem(self.units, existing=False)
        with self.assertRaisesRegex(RuntimeError, 'old V2 calibration frame'):
            installer.rollback(self.root, system)
        self.assertEqual(system.calls, [])


if __name__ == '__main__':
    unittest.main()
