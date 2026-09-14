import json
from pathlib import Path
import tempfile
import unittest

from daemon_support import DaemonTestCase, client, daemon, request


class DaemonTests(DaemonTestCase):
    def test_help_and_invalid_arguments_use_documented_exit_codes(self):
        with tempfile.TemporaryDirectory(prefix='orion-client-test-', dir='/tmp') as temporary:
            path = Path(temporary) / 'absent.sock'
            for arguments in [['--unknown'], ['--goto'], ['--enable', '--disable'],
                              ['--light', '256', '0', '0', '0']]:
                with self.subTest(arguments=arguments):
                    self.assertEqual(client(path, *arguments).returncode, 2)
            help_result = client(path, '--help')
            self.assertEqual(help_result.returncode, 0)
            self.assertIn('oriond --serve', help_result.stdout)

    def test_maintenance_daemon_dispatches_commands_and_waits_for_completion(self):
        with daemon() as path:
            self.assertEqual(request(path, 'status')['mode'], 'observe')
            self.assertFalse(request(path, 'character status')['character']['enabled'])
            self.assertFalse(request(path, 'enable')['ok'])
            for command, field, expected in [('pose list', 'poses', 'home'),
                                             ('motion list', 'motions', 'idle_breathe'),
                                             ('scene list', 'scenes', 'deployment_smoke')]:
                self.assertIn(expected, request(path, command)[field])
            self.assertEqual(len(request(path, 'joint limits')['joints']), 5)
            for command in ['scene reload', 'asset reload', 'lamp 1 2 3 4',
                            'lamp-effect {"brightness":0.5,"effect":"solid"}']:
                self.assertTrue(request(path, command)['ok'], command)
            for command in ['lamp 256 0 0 0', 'lamp-effect {"brightness":2}',
                            'voice invalid wake', 'scene preview {}',
                            'speech append', 'unknown-command']:
                self.assertFalse(request(path, command)['ok'], command)
            session = 'b' * 32
            self.assertTrue(request(path, f'voice {session} wake')['ok'])
            self.assertTrue(request(path, f'voice {session} endpoint')['ok'])
            self.assertTrue(request(path, f'voice {session} cancel')['ok'])
            self.assertFalse(request(path, f'speech stream unused {session}')['ok'])
            self.assertTrue(request(path, 'configure')['ok'])
            self.assertTrue(request(path, 'enable')['ok'])
            self.assertEqual(client(path, '--configure').returncode, 3)
            for arguments in [('--goto', 'home', '--duration', '0.7', '--wait'),
                              ('--run-scene', 'deployment_smoke', '--wait')]:
                completed = client(path, *arguments)
                self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
                self.assertEqual(json.loads(completed.stdout.splitlines()[-1])['state'], 'completed')
            self.assertEqual(request(path, 'status')['mode'], 'holding')
            self.assertTrue(request(path, 'disable')['ok'])

    def test_default_startup_enables_character_and_stop_finishes(self):
        with daemon(automatic_character=True) as path:
            self.wait_for_character(path, 'home_idle', True)
            self.assertTrue(request(path, 'character stop')['ok'])
            self.wait_for_character(path, 'off', False)


if __name__ == '__main__':
    unittest.main(verbosity=2)
