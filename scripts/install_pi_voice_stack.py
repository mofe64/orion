#!/usr/bin/env python3
"""Activate a prepared full Pi release, preserving configuration and one rollback."""
import argparse
import contextlib
import fcntl
import json
import os
from pathlib import Path
import pwd
import re
import shutil
import signal
import socket
import subprocess
import tempfile
import time
import urllib.request
import uuid

from pi_service_config import SERVICES, STOP_ORDER, read_env, render_plan


def atomic_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix('.tmp')
    with temporary.open('w') as output:
        output.write(json.dumps(value, indent=2))
        output.flush()
        os.fsync(output.fileno())
    temporary.chmod(0o600)
    temporary.replace(path)


@contextlib.contextmanager
def deployment_lock(root):
    root.mkdir(parents=True, exist_ok=True)
    with (root / 'deployment.lock').open('a') as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise RuntimeError('Another Pi deployment is in progress') from None
        yield


class System:
    def run(self, *args, **kwargs):
        return subprocess.run([str(a) for a in args], check=True, **kwargs)

    def state(self, service):
        result = subprocess.run(['systemctl', 'show', service, '--property=LoadState',
            '--property=ActiveState', '--property=UnitFileState'], capture_output=True, text=True)
        values = dict(line.split('=', 1) for line in result.stdout.splitlines() if '=' in line)
        if values.get('LoadState') == 'not-found':
            return {'active': False, 'enabled': 'not-found'}
        if result.returncode or values.get('LoadState') != 'loaded':
            raise RuntimeError(f'Cannot inspect {service}; it may be masked or misconfigured')
        enabled = values.get('UnitFileState', '')
        if enabled not in ('enabled', 'enabled-runtime', 'disabled', 'static', 'indirect'):
            raise RuntimeError(f'Unsupported {service} enablement: {enabled}')
        if values.get('ActiveState') not in ('active', 'inactive', 'failed'):
            raise RuntimeError(f'{service} is changing state; retry after it settles')
        return {'active': values['ActiveState'] == 'active', 'enabled': enabled}

    def stop(self, name):
        if self.state(name)['enabled'] != 'not-found':
            self.run('sudo', 'systemctl', 'stop', name)

    def start(self, name):
        self.run('sudo', 'systemctl', 'start', name)

    def enable(self, name, state):
        if state == 'enabled-runtime':
            self.run('sudo', 'systemctl', 'enable', '--runtime', name)
        elif state == 'enabled':
            self.run('sudo', 'systemctl', 'enable', name)
        elif state in ('disabled', 'not-found'):
            # Restore disabled/missing units without following symlinks into another release.
            self.run('sudo', 'systemctl', 'disable', name)

    def reload(self):
        self.run('sudo', 'systemctl', 'daemon-reload')

    def write(self, path, data, mode):
        with tempfile.NamedTemporaryFile() as source:
            source.write(data); source.flush()
            # install to a sibling followed by rename keeps readers from seeing partial files.
            temporary = path.with_name(path.name + '.orion-installing')
            self.run('sudo', 'install', '-D', '-m', f'{mode:o}', source.name, temporary)
            if not str(path).startswith('/etc/'):
                self.run('sudo', 'chown', f'{os.getuid()}:{os.getgid()}', temporary)
            self.run('sudo', 'mv', '-f', temporary, path)

    def remove(self, path):
        self.run('sudo', 'rm', '-f', path)

    def rest_runtime(self):
        """Confirm torque is off before systemd can terminate the hardware owner."""
        pid = self.run('systemctl', 'show', 'oriond', '--property=MainPID', '--value', capture_output=True, text=True).stdout.strip()
        if not pid.isdigit() or int(pid) == 0:
            raise RuntimeError('Cannot identify the current runtime for a safe release switch')
        binary = f'/proc/{pid}/exe'
        arguments = Path(f'/proc/{pid}/cmdline').read_bytes().decode().split('\0')
        client = [binary, '--socket', option(arguments, '--socket', '/tmp/oriond.sock')]
        status = json.loads(self.run(*client, '--status', capture_output=True, text=True).stdout)
        if status.get('torque_enabled'):
            for command, inactive in (
                ('--stop-scene', {'ok': False, 'error': 'No scene is active.'}),
                ('--stop-speech', {'ok': False, 'error': 'No speech run is active.'}),
                ('--stop', {'ok': False, 'command': 'stop', 'error': 'no movement is active'}),
            ):
                try:
                    self.run(*client, command, capture_output=True, text=True)
                except subprocess.CalledProcessError as error:
                    # Only an exact already-stopped rejection is harmless.
                    try:
                        response = json.loads(error.stdout or '')
                    except (ValueError, TypeError):
                        raise error
                    if error.returncode != 3 or response != inactive:
                        raise
            self.run(*client, '--goto', 'rest', '--duration', '3.0', '--wait', capture_output=True)
            self.run(*client, '--disable', capture_output=True)
        status = json.loads(self.run(*client, '--status', capture_output=True, text=True).stdout)
        if status.get('torque_enabled') is not False:
            raise RuntimeError('Mechanical rest/torque-off was not confirmed; runtime was left running')

    def ready(self, release, home, timeout=180):
        metadata = json.loads((release / 'release.json').read_text())
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                if not all(self.state(name)['active'] for name in SERVICES):
                    raise RuntimeError('Services are still starting')
                # Read the running gateway's arguments, including locally chosen port/token/socket.
                pid = self.run('systemctl', 'show', 'orion-studio-gateway', '--property=MainPID', '--value', capture_output=True, text=True).stdout.strip()
                if not pid.isdigit() or int(pid) == 0:
                    raise RuntimeError('Gateway has no running process')
                arguments = Path(f'/proc/{pid}/cmdline').read_bytes().decode().split('\0')
                runtime_socket = option(arguments, '--socket', '/tmp/oriond.sock')
                port = int(option(arguments, '--port', '7447'))
                token_file = Path(option(arguments, '--token-file', str(home / '.config/orion/studio-token')))
                status = control(home, {'method': 'status'})
                observation = control(home, {'method': 'observe'})
                ready = next((e for e in observation.get('events', []) if e.get('type') == 'ready'), {})
                expected_provider, expected_model = expected_tts(home)
                saved_settings = home / '.config/orion/voice-settings.json'
                settings = json.loads(saved_settings.read_text()) if saved_settings.exists() else {}
                expected_agent = settings.get('model', settings.get('agent_model', 'gpt-6-luna'))
                expected_effort = settings.get('effort', settings.get('agent_effort', 'medium'))
                listener_pid = self.run('systemctl', 'show', 'orion-listener', '--property=MainPID', '--value',
                                        capture_output=True, text=True).stdout.strip()
                if not listener_pid.isdigit() or int(listener_pid) == 0:
                    raise RuntimeError('Listener has no running process')
                listener_args = Path(f'/proc/{listener_pid}/cmdline').read_bytes().decode().split('\0')
                expected_wake_model = Path(option(listener_args, '--wake-model',
                    str(release / 'voice/models/wake/hey_orion_trained_080.rpw'))).name
                expected_wake_threshold = float(option(listener_args, '--threshold', '0.80'))
                if not (status.get('coordinator_running') and not status.get('error') and
                        status.get('project_root') == str(release) and status.get('revision') == metadata['revision'] and
                        ready.get('asr', {}).get('provider') == 'qwen3-asr' and
                        ready.get('tts', {}).get('provider') == expected_provider and
                        ready.get('tts', {}).get('model') == expected_model and
                        ready.get('agent', {}).get('model') == expected_agent and
                        ready.get('agent', {}).get('effort') == expected_effort and
                        ready.get('wake', {}).get('model') == expected_wake_model and
                        ready.get('wake', {}).get('threshold') == expected_wake_threshold):
                    raise RuntimeError('The requested release is not speech-ready')
                runtime = json.loads(self.run(release / 'runtime/target/release/oriond', '--socket', runtime_socket, '--status', capture_output=True, text=True, timeout=5).stdout)
                if runtime.get('build_revision') != metadata['revision']:
                    raise RuntimeError('The old runtime is still active')
                # The gateway must reach the same authenticated voice host.
                token = token_file.read_text().strip()
                request = urllib.request.Request(f'http://127.0.0.1:{port}/api/v2/voice/request',
                    data=b'{"method":"status"}', headers={'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'})
                with urllib.request.urlopen(request, timeout=3) as response:
                    gateway = json.load(response)
                if gateway.get('pid') != status.get('pid'):
                    raise RuntimeError('Gateway is connected to a different release')
                return
            except (OSError, ValueError, RuntimeError, subprocess.SubprocessError):
                time.sleep(.5)
        raise RuntimeError('Pi release did not become ready: check runtime, Qwen, TTS, Codex and gateway logs')


def expected_tts(home):
    saved = home / '.config/orion/voice-settings.json'
    if saved.exists():
        settings = json.loads(saved.read_text())
        model = ('piper-alba-medium' if str(settings.get('ttsModel', '')).startswith('pocket-')
                 else settings.get('ttsPath') or settings.get('ttsModel') or 'piper-alba-medium')
        if model == '~' or model.startswith('~/'):
            model = str(home / model.removeprefix('~/'))
    else:
        environment = home / '.config/orion/voice-stack.env'
        values = read_env(environment.read_text()) if environment.exists() else {}
        model = values.get('ORION_STUDIO_TTS_MODEL', 'piper-alba-medium')
    if model.startswith('pocket-'):
        model = 'piper-alba-medium'
    if model != 'piper-alba-medium' and not (Path(model) / 'en_GB-alba-medium.onnx').is_file():
        raise RuntimeError(f'Unsupported Pi speech model: {model}')
    return 'piper-tts', model


def option(arguments, flag, default):
    for index, value in enumerate(arguments):
        if value == flag:
            return arguments[index + 1]
        if value.startswith(flag + '='):
            return value.partition('=')[2]
    return default


def control(home, request):
    directory = home / '.local/share/orion/studio-service'
    info = json.loads((directory / 'connection.json').read_text())
    host, port = info['address'].rsplit(':', 1)
    if host != '127.0.0.1' or info['protocol'] != 1:
        raise RuntimeError('Invalid local service discovery')
    with socket.create_connection((host, int(port)), timeout=3) as connection:
        connection.sendall(json.dumps({'protocol': 1, 'token': info['token'], 'request': request}).encode() + b'\n')
        with connection.makefile('rb') as stream:
            raw = stream.readline(1024 * 1024 + 1)
    if len(raw) > 1024 * 1024:
        raise RuntimeError('Oversized service response')
    result = json.loads(raw)
    if not result.get('ok'):
        raise RuntimeError('Service request failed')
    return result['result']


def snapshot(root, files, system):
    folder = root / 'transactions' / uuid.uuid4().hex
    folder.mkdir(parents=True, mode=0o700)
    data = {'version': 2, 'status': 'prepared', 'files': [],
            'services': {name: system.state(name) for name in SERVICES}}
    for index, path in enumerate(files):
        if path.is_symlink():
            raise RuntimeError(f'Refusing to replace a symlink: {path}')
        backup = folder / str(index)
        if path.exists():
            shutil.copy2(path, backup)
        data['files'].append({'path': str(path), 'backup': str(backup) if path.exists() else None,
                              'mode': path.stat().st_mode & 0o777 if path.exists() else None})
    atomic_json(folder / 'state.json', data)
    return folder, data


def restore(folder, data, system):
    # A new runtime may be holding torque after partial activation.
    for name in STOP_ORDER:
        if name == 'oriond' and system.state(name)['active']:
            system.rest_runtime()
        system.stop(name)
    # Disable newly-created units while their definitions still exist, removing wants links.
    for name, old in data['services'].items():
        if old['enabled'] == 'not-found' and system.state(name)['enabled'] != 'not-found':
            system.enable(name, 'not-found')
    for entry in reversed(data['files']):
        path = Path(entry['path'])
        if entry['backup']:
            system.write(path, Path(entry['backup']).read_bytes(), entry['mode'])
        else:
            system.remove(path)
    system.reload()
    for name in SERVICES:
        old = data['services'][name]
        # No enable/disable request for a now-missing service: its unit file is already gone.
        if system.state(name)['enabled'] != 'not-found':
            system.enable(name, old['enabled'])
        if old['active']:
            system.start(name)
    # Starting oriond pulls in its listener through Wants=. Restore intentionally
    # stopped companions after dependencies have had a chance to start them.
    for name in STOP_ORDER:
        if not data['services'][name]['active'] and system.state(name)['active']:
            if name == 'oriond':
                system.rest_runtime()
            system.stop(name)
    data['status'] = 'rolled_back'
    atomic_json(folder / 'state.json', data)


def activate(root, release, home, files, system):
    installed = root / 'installation.json'
    pending = root / 'pending-installation.json'
    if pending.exists():
        raise RuntimeError('An interrupted deployment needs recovery; run this installer with --rollback')
    previous = json.loads(installed.read_text()) if installed.exists() else None
    if previous and previous.get('version') == 2 and previous.get('release') == str(release):
        raise RuntimeError('Release is already installed; build a new release directory for an update')
    folder, data = snapshot(root, [*files, installed], system)
    data['release'] = str(release)
    atomic_json(folder / 'state.json', data)
    atomic_json(pending, {'transaction': str(folder)})
    switched = False
    try:
        # Quiesce companions first. Failure to rest never forces termination of the runtime.
        for name in STOP_ORDER[:-1]:
            system.stop(name)
        if data['services']['oriond']['active']:
            system.rest_runtime()
        switched = True
        data['status'] = 'switching'; atomic_json(folder / 'state.json', data)
        system.stop('oriond')
        for path, value in files.items():
            system.write(path, value.encode(), 0o644 if str(path).startswith('/etc/') else 0o600)
        system.reload()
        for name in SERVICES:
            if data['services'][name]['enabled'] == 'not-found':
                system.enable(name, 'enabled')
            system.start(name)
        system.ready(release, home)
        atomic_json(installed, {'version': 2, 'release': str(release), 'rollback': str(folder)})
        data['status'] = 'committed'; atomic_json(folder / 'state.json', data)
    except BaseException:
        if switched:
            try:
                restore(folder, data, system)
            except BaseException:
                data['status'] = 'rollback_failed'; atomic_json(folder / 'state.json', data)
                raise RuntimeError(f'Recovery could not finish; preserve {folder} and run --rollback') from None
        else:
            for name in SERVICES[1:]:
                if data['services'][name]['active']:
                    system.start(name)
            for name in STOP_ORDER[:-1]:
                if not data['services'][name]['active']:
                    system.stop(name)
            data['status'] = 'rolled_back'; atomic_json(folder / 'state.json', data)
        pending.unlink(missing_ok=True)
        raise
    pending.unlink(missing_ok=True)
    # Retain exactly one transaction owned by this installer. Never prune release/source directories.
    for old in folder.parent.iterdir():
        if old.is_dir() and not old.is_symlink() and re.fullmatch(r'[0-9a-f]{32}', old.name) and old != folder:
            try:
                shutil.rmtree(old)
            except OSError as error:
                print(f'Release is active; could not remove old rollback {old}: {error}')
    return folder


def rollback(root, system):
    pending = root / 'pending-installation.json'
    pointer = json.loads((pending if pending.exists() else root / 'installation.json').read_text())
    folder = Path(pointer.get('transaction') or pointer.get('rollback') or '')
    if not folder.is_absolute() or folder.parent != root / 'transactions':
        raise RuntimeError('No recent transaction rollback exists; legacy pre-installation rollback is not an update rollback')
    data = json.loads((folder / 'state.json').read_text())
    already_restored = data['status'] == 'rolled_back'
    if not already_restored:
        atomic_json(pending, {'transaction': str(folder)})
        restore(folder, data, system)
    # A manual rollback consumes this one-level recovery point. An older transaction
    # may have been pruned; never leave an installation pointer to that missing backup.
    installed = root / 'installation.json'
    if installed.exists():
        previous = json.loads(installed.read_text())
        if previous.get('version') == 2:
            previous['rollback'] = str(folder)
            atomic_json(installed, previous)
    pending.unlink(missing_ok=True)
    print('The previous installation has already been restored.' if already_restored else
          'Restored the immediately previous service configuration and running state.')


def preflight(release, home, files, system):
    required = ['release.json', 'runtime/target/release/oriond', 'runtime/target/release/orion-trajectory',
                'orion-service/target/release/orion-service', 'speech/.venv/bin/python', 'voice/.venv/bin/orion-listener']
    for name in required:
        if not (release / name).is_file():
            raise RuntimeError(f'Prepare the complete release first: {name}')
    if not (home / '.config/orion/servo_calibration.json').is_file() or not (home / '.config/orion/studio-token').is_file():
        raise RuntimeError('Install hardware calibration and pairing token before activating voice')
    environment = {**os.environ, **read_env(files[home / '.config/orion/voice-stack.env']),
                   'ORION_PROJECT_ROOT': str(release), 'ORION_ONBOARD': '1', 'ORION_SPEECH_BACKEND': 'pi'}
    system.run(environment['ORION_STUDIO_CODEX_BIN'], 'login', 'status')
    system.run(release / 'orion-service/target/release/orion-service', 'check', env=environment)


def validate_catalog(release, project, calibration, system):
    # Compile against the operator's existing catalog without moving the robot or
    # migrating assets. Incompatible assets must fail before service activation.
    system.run(release / 'runtime/target/release/orion-trajectory',
        '--motion', 'look_at_left_expressive', '--start-pose', 'attentive',
        '--pose-file', project / 'motion/config/poses.yaml',
        '--motions-directory', project / 'motion/motions', '--calibration', calibration,
        stdout=subprocess.DEVNULL)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path.home() / '.local/share/orion/voice-stack')
    parser.add_argument('--release', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--runtime-project', type=Path, default=Path.home() / 'dev/orion')
    parser.add_argument('--rollback', action='store_true')
    parser.add_argument('--plan', action='store_true', help='Show affected paths without modifying files or services')
    args = parser.parse_args()
    root, release, home = args.root.resolve(), args.release.resolve(), Path.home()
    if args.plan:
        files = render_plan(release, root, args.runtime_project.resolve(), home, pwd.getpwuid(os.getuid()).pw_name)
        print(json.dumps({'release': str(release), 'files': [str(p) for p in files]}, indent=2))
        return
    if os.uname().machine != 'aarch64' or os.geteuid() == 0:
        raise RuntimeError('Run as the Pi user on 64-bit Linux; the installer requests sudo only when needed')
    system = System()
    with deployment_lock(root):
        system.run('sudo', '-v')
        if args.rollback:
            rollback(root, system); return
        files = render_plan(release, root, args.runtime_project.resolve(), home, pwd.getpwuid(os.getuid()).pw_name)
        preflight(release, home, files, system)
        validate_catalog(release, args.runtime_project.resolve(), home / '.config/orion/servo_calibration.json', system)
        def interrupted(signum, frame):
            raise KeyboardInterrupt('Deployment interrupted')
        signal.signal(signal.SIGTERM, interrupted)
        folder = activate(root, release, home, files, system)
        print(f'Complete Pi release is ready. Previous installation can be restored from {folder}.')


if __name__ == '__main__':
    main()
