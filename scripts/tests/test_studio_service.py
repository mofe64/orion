"""Exercise release rollback and launchd configuration without touching the robot."""
import importlib.util
import os
from pathlib import Path
import plistlib
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch
import uuid

SCRIPT = Path(__file__).resolve().parents[1] / 'studio_service.py'
spec = importlib.util.spec_from_file_location('studio_service', SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ServiceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='orion service ')
        self.addCleanup(self.temporary.cleanup)
        root = Path(self.temporary.name)
        self.environment = patch.dict(os.environ, {
            'ORION_STUDIO_SERVICE_HOME': str(root / 'service'),
            'ORION_STUDIO_LAUNCH_AGENTS': str(root / 'agents'),
            'ORION_STUDIO_LAUNCHD_LABEL': 'org.orion.test.' + uuid.uuid4().hex,
        })
        self.environment.start(); self.addCleanup(self.environment.stop)
        self.service = module.Service()
        self.service.home.mkdir()
        self.old = self.service.home / 'releases/old'; self.old.mkdir(parents=True)
        self.new = self.service.home / 'releases/new'; self.new.mkdir()
        self.service.switch(self.old)
        self.service.plist.parent.mkdir()
        self.service.plist.write_bytes(b'previous registration')
        (self.service.home / 'installed').touch()

    def test_failed_preparation_keeps_running_release(self):
        with patch.object(self.service, 'prepare', side_effect=RuntimeError('build failed')), \
             patch.object(self.service, 'stop') as stop:
            with self.assertRaisesRegex(RuntimeError, 'build failed'):
                self.service.update()
        stop.assert_not_called()
        self.assertEqual(self.service.current.resolve(), self.old)
        self.assertEqual(self.service.plist.read_bytes(), b'previous registration')

    def test_failed_start_restores_release_registration_and_running_state(self):
        calls = []
        def start():
            calls.append(self.service.current.resolve())
            if len(calls) == 1:
                raise RuntimeError('new binary failed')
            return {'revision': 'old'}
        with patch.object(self.service, 'prepare', return_value=(self.new, 'new')), \
             patch.object(self.service, 'loaded', return_value=True), \
             patch.object(self.service, 'stop') as stop, \
             patch.object(self.service, 'write_plist', side_effect=lambda _: self.service.plist.write_bytes(b'new registration')), \
             patch.object(self.service, 'start', side_effect=start):
            with self.assertRaisesRegex(RuntimeError, 'new binary failed'):
                self.service.update()
        self.assertEqual(calls, [self.new, self.old])
        self.assertEqual(stop.call_count, 2)
        self.assertEqual(self.service.plist.read_bytes(), b'previous registration')
        self.assertTrue((self.service.home / 'installed').exists())

    def test_success_switches_only_after_preparation_and_keeps_registration(self):
        order = []
        def prepare(_):
            self.assertEqual(self.service.current.resolve(), self.old)
            order.append('prepare')
            return self.new, 'new'
        def stop():
            self.assertEqual(self.service.current.resolve(), self.old)
            order.append('stop')
        def start():
            self.assertEqual(self.service.current.resolve(), self.new)
            self.assertTrue((self.service.home / 'installed').exists())
            order.append('start')
            return {'revision': 'new', 'coordinator_running': True}
        with patch.object(self.service, 'prepare', side_effect=prepare), \
             patch.object(self.service, 'loaded', return_value=True), \
             patch.object(self.service, 'stop', side_effect=stop), \
             patch.object(self.service, 'write_plist'), \
             patch.object(self.service, 'start', side_effect=start):
            self.service.update()
        self.assertEqual(order, ['prepare', 'stop', 'start'])
        self.assertTrue(self.old.exists())

    def test_failed_first_install_removes_registration(self):
        self.service.current.unlink(); self.service.plist.unlink()
        (self.service.home / 'installed').unlink()
        with patch.object(self.service, 'prepare', return_value=(self.new, 'new')), \
             patch.object(self.service, 'loaded', return_value=False), \
             patch.object(self.service, 'stop'), \
             patch.object(self.service, 'write_plist', side_effect=lambda _: self.service.plist.write_bytes(b'new registration')), \
             patch.object(self.service, 'start', side_effect=RuntimeError('failed')):
            with self.assertRaises(RuntimeError):
                self.service.update()
        self.assertFalse(self.service.current.exists())
        self.assertFalse(self.service.plist.exists())
        self.assertFalse((self.service.home / 'installed').exists())

    def test_transaction_rejects_concurrent_update(self):
        with self.service.transaction():
            with self.assertRaisesRegex(RuntimeError, 'Another Studio'):
                with module.Service().transaction():
                    self.fail('Second updater acquired the lock')

    def test_dirty_pull_does_not_fetch_or_stop(self):
        with patch.object(module, 'run', return_value=subprocess.CompletedProcess([], 0, ' M agent/src/lib.rs\n')) as run:
            with self.assertRaisesRegex(RuntimeError, 'clean checkout'):
                self.service.prepare(True)
        self.assertEqual(run.call_count, 1)
        self.assertEqual(self.service.current.resolve(), self.old)

    def test_launchagent_preserves_paths_and_login_restart_without_credentials(self):
        with patch.dict(os.environ, {'ORION_STUDIO_CODEX_BIN': '/Applications/Codex App/codex',
                                    'ORION_PI_TOKEN': 'must-not-be-copied'}), \
             patch.object(module, 'run'):
            self.service.write_plist('new')
        value = plistlib.loads(self.service.plist.read_bytes())
        self.assertTrue(value['RunAtLoad']); self.assertTrue(value['KeepAlive'])
        self.assertEqual(value['ProgramArguments'], [str(self.service.current / 'bin/orion-studio-headless'), 'serve'])
        self.assertEqual(value['EnvironmentVariables']['ORION_STUDIO_CODEX_BIN'], '/Applications/Codex App/codex')
        self.assertNotIn('must-not-be-copied', self.service.plist.read_text())
        self.assertEqual(self.service.plist.stat().st_mode & 0o777, 0o600)

    @unittest.skipUnless(sys.platform == 'darwin' and os.environ.get('ORION_TEST_LAUNCHD') == '1',
                         'Opt-in macOS login-session integration test')
    def test_real_launchd_restarts_crashes_and_supports_stop_start(self):
        binary = module.ROOT / 'studio-service/target/debug/orion-studio-headless'
        self.assertTrue(binary.is_file(), 'Build the headless binary first')
        import shlex
        wrapper = self.old / 'bin/orion-studio-headless'; wrapper.parent.mkdir()
        wrapper.write_text('#!/bin/sh\nif [ "$1" = serve ]; then\nexec ' + shlex.quote(str(binary)) +
                           ' serve --no-autostart\nfi\nexec ' + shlex.quote(str(binary)) + ' "$@"\n')
        wrapper.chmod(0o755)
        self.service.write_plist('test')
        self.addCleanup(self.service.stop)
        first = self.service.start()
        self.assertEqual(first['revision'], 'test')
        os.kill(first['pid'], signal.SIGKILL)
        deadline = time.monotonic() + 25
        second = None
        while time.monotonic() < deadline:
            second = self.service.status()
            if second and second['pid'] != first['pid']:
                break
            time.sleep(.2)
        self.assertIsNotNone(second)
        self.assertNotEqual(second['pid'], first['pid'])
        self.service.stop()
        self.assertFalse(self.service.loaded())
        self.assertIsNone(self.service.status())
        third = self.service.start()
        self.assertNotEqual(third['pid'], second['pid'])
        self.assertTrue(self.service.plist.exists())


if __name__ == '__main__':
    unittest.main()
