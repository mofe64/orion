import contextlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import unittest
import uuid
import wave


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
        stream.sendall(command.encode() + b'\n')
        received = bytearray()
        while not received.endswith(b'\n'):
            chunk = stream.recv(65536)
            if not chunk:
                break
            received.extend(chunk)
        return json.loads(received)


@contextlib.contextmanager
def daemon(automatic_character=False, rest_after_seconds=600, following=False):
    with tempfile.TemporaryDirectory(prefix='orion-daemon-test-', dir='/tmp') as temporary:
        path = Path(temporary) / 'daemon.sock'
        arguments = [str(BINARY), '--serve', '--backend', 'mujoco', '--socket', str(path),
                     '--start-pose', 'home', '--python', str(PACKAGE / 'tests/fixtures/following_driver.py' if following else ROOT / '.venv/bin/python'),
                     '--rest-after-seconds', str(rest_after_seconds)]
        if not automatic_character:
            arguments += ['--character-on-start', 'off']
        environment = os.environ.copy()
        if following:
            environment['ORION_TEST_DRIVER_CONTROL'] = str(Path(temporary) / 'driver-control.json')
            environment['ORION_TEST_DRIVER_EVENTS'] = str(Path(temporary) / 'driver-events.jsonl')
        with tempfile.TemporaryFile(mode='w+') as output:
            process = subprocess.Popen(arguments, cwd=ROOT, stdout=output, stderr=output, env=environment)
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


class DaemonTestCase(unittest.TestCase):
    def ok(self, path, command):
        response = request(path, command)
        self.assertTrue(response['ok'], f'{command}: {response}')
        return response

    def voice(self, path, session, *events):
        for event in events:
            self.ok(path, f'voice {session} {event}')

    def driver_control(self, path, **settings):
        control = path.parent / 'driver-control.json'
        temporary = control.with_suffix('.tmp')
        temporary.write_text(json.dumps(settings))
        temporary.replace(control)

    def torque_events(self, path):
        return [json.loads(line)['command'] for line in
                (path.parent / 'driver-events.jsonl').read_text().splitlines()]

    def wait_for(self, path, command, predicate, timeout=18):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            snapshot = request(path, command)
            if predicate(snapshot):
                return snapshot
            time.sleep(.02)
        self.fail(f'{command} did not reach expected state: {snapshot}')

    @contextlib.contextmanager
    def reply_file(self):
        spool = Path('/tmp/orion-speech-spool')
        spool.mkdir(exist_ok=True)
        identifier = 'rest-test-' + uuid.uuid4().hex
        path = spool / f'{identifier}.wav'
        with wave.open(str(path), 'wb') as wav:
            wav.setnchannels(1)
            wav.setsampwidth(2)
            wav.setframerate(24000)
            wav.writeframes(b'\0\0' * 24000)
        try:
            yield identifier, path
        finally:
            path.unlink(missing_ok=True)

    @classmethod
    def setUpClass(cls):
        if not BINARY.is_file():
            raise AssertionError(f'Build {BINARY} before running daemon tests')


    def wait_for_rest(self, path, state):
        deadline = time.monotonic() + 18
        while time.monotonic() < deadline:
            snapshot = request(path, 'character status')
            if snapshot['rest']['state'] == state:
                return snapshot
            if snapshot['rest']['state'] == 'fault':
                self.fail(f'Rest lifecycle failed: {snapshot}')
            time.sleep(.02)
        self.fail(f'Rest lifecycle did not reach {state}: {snapshot}')


    def wait_for_character(self, path, expected_state, enabled):
        deadline = time.monotonic() + 12
        while time.monotonic() < deadline:
            status = request(path, 'character status')['character']
            if status['state'] == expected_state and status['enabled'] == enabled:
                return
            time.sleep(0.02)
        self.fail(f'Character did not reach {expected_state}: {status}')
