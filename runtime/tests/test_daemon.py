import contextlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import unittest


PACKAGE = Path(__file__).resolve().parents[1]
ROOT = PACKAGE.parent
BIN_DIRECTORY = Path(os.environ.get('ORION_TEST_BIN_DIR', str(PACKAGE / 'target/debug')))
BINARY = BIN_DIRECTORY / 'oriond'


def client(path, *arguments):
    return subprocess.run(
        [str(BINARY), '--socket', str(path), *arguments], cwd=ROOT,
        capture_output=True, text=True, timeout=40,
    )


def request(path, command):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as stream:
        stream.settimeout(5)
        stream.connect(str(path))
        stream.sendall(command.encode())
        received = bytearray()
        while not received.endswith(b'\n'):
            chunk = stream.recv(65536)
            if not chunk:
                break
            received.extend(chunk)
        return json.loads(received)


@contextlib.contextmanager
def daemon(automatic_character=False):
    with tempfile.TemporaryDirectory(prefix='orion-daemon-test-', dir='/tmp') as temporary:
        path = Path(temporary) / 'daemon.sock'
        arguments = [str(BINARY), '--serve', '--backend', 'mujoco', '--socket', str(path),
                     '--start-pose', 'home', '--python', str(ROOT / '.venv/bin/python')]
        if not automatic_character:
            arguments += ['--character-on-start', 'off']
        with tempfile.TemporaryFile(mode='w+') as output:
            process = subprocess.Popen(arguments, cwd=ROOT, stdout=output, stderr=output)
            try:
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    if process.poll() is not None:
                        output.seek(0)
                        raise AssertionError(output.read())
                    if path.exists():
                        request(path, 'status')
                        break
                    time.sleep(0.02)
                else:
                    raise AssertionError('Daemon did not create its socket')
                yield path
            finally:
                if process.poll() is None:
                    process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)


class DaemonTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not BINARY.is_file():
            raise AssertionError(f'Build {BINARY} before running daemon tests')

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

    def wait_for_character(self, path, expected_state, enabled):
        deadline = time.monotonic() + 12
        while time.monotonic() < deadline:
            status = request(path, 'character status')['character']
            if status['state'] == expected_state and status['enabled'] == enabled:
                return
            time.sleep(0.02)
        self.fail(f'Character did not reach {expected_state}: {status}')


if __name__ == '__main__':
    unittest.main(verbosity=2)
