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

from pi_service_config import ACTIVE_WAKE_THRESHOLD, SERVICES, STOP_ORDER, read_env, render_plan
from pi_catalog import catalog_plan
from pi_hardware import profile, release_hardware, calibration_path


def atomic_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix('.tmp')
    with temporary.open('w') as output:
        output.write(json.dumps(value, indent=2))
        output.flush()
        os.fsync(output.fileno())
    temporary.chmod(0o600)
    temporary.replace(path)


def failure_details(error):
    details = f'{type(error).__name__}: {error}'
    if isinstance(error, subprocess.CalledProcessError):
        for name in ('stdout', 'stderr'):
            value = getattr(error, name)
            if isinstance(value, bytes):
                value = value.decode(errors='replace')
            if value and value.strip():
                details += f'\n{name}: {value.strip()}'
    return details


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

    def state(self, service, settle_timeout=30):
        # A crashed oriond spends RestartSec in "activating"; wait for systemd
        # to finish that transition instead of abandoning recovery.
        deadline = time.monotonic() + settle_timeout
        while True:
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
            active = values.get('ActiveState')
            if active in ('active', 'inactive', 'failed'):
                return {'active': active == 'active', 'enabled': enabled}
            if time.monotonic() >= deadline:
                raise RuntimeError(f'{service} stayed {active} for {settle_timeout}s; it may be crash-looping.'
                                   f'{journal_tail(service)}')
            time.sleep(.5)

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
        if not str(path).startswith('/etc/'):
            # `install -D` would create missing parents as root, and a
            # root-owned ~/.local/share/orion/studio-service stops orion-service
            # from making its own directory private.
            path.parent.mkdir(parents=True, exist_ok=True)
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

    def foreign_runtimes(self):
        """oriond processes that systemd does not own, such as a hand-started copy."""
        try:
            main = subprocess.run(['systemctl', 'show', 'oriond', '--property=MainPID', '--value'],
                                  capture_output=True, text=True).stdout.strip()
            found = subprocess.run(['pgrep', '-x', 'oriond'], capture_output=True, text=True).stdout.split()
        except OSError:
            return []
        return [pid for pid in found if pid.isdigit() and pid != main]

    def runtime_client(self):
        """Use the running hardware owner's binary and configured socket."""
        pid = self.run('systemctl', 'show', 'oriond', '--property=MainPID', '--value', capture_output=True, text=True).stdout.strip()
        if not pid.isdigit() or int(pid) == 0:
            raise RuntimeError('Cannot identify the current hardware runtime')
        arguments = Path(f'/proc/{pid}/cmdline').read_bytes().decode().split('\0')
        # Resolve the executable now: /proc/<pid>/exe disappears if systemd
        # restarts oriond, while the release binary it points to remains.
        binary = f'/proc/{pid}/exe'
        try:
            target = os.readlink(binary)
        except OSError:
            target = None
        if target and os.path.isabs(target) and not target.endswith(' (deleted)'):
            binary = target
        return [binary, '--socket', option(arguments, '--socket', '/tmp/oriond.sock')]

    def wait_runtime_client(self, revision=None, timeout=20):
        deadline = time.monotonic() + timeout
        while True:
            try:
                client = self.runtime_client()
                status = json.loads(self.run(*client, '--status', capture_output=True, text=True, timeout=5).stdout)
                if revision is not None and status.get('build_revision') != revision:
                    raise RuntimeError('The requested runtime has not started')
                return client, status
            except (OSError, ValueError, RuntimeError, subprocess.SubprocessError):
                if time.monotonic() >= deadline:
                    raise RuntimeError('Runtime did not become available for deployment movement') from None
                time.sleep(.1)

    def stop_playback(self, client):
        for command, inactive in (
            ('--stop-scene', {'ok': False, 'error': 'No scene is active.'}),
            ('--stop-speech', {'ok': False, 'error': 'No speech run is active.'}),
            ('--stop', {'ok': False, 'command': 'stop', 'error': 'no movement is active'}),
        ):
            try:
                self.run(*client, command, capture_output=True, text=True, timeout=10)
            except subprocess.CalledProcessError as error:
                # Only an exact already-stopped rejection is harmless.
                try:
                    response = json.loads(error.stdout or '')
                except (ValueError, TypeError):
                    raise error
                if error.returncode != 3 or response != inactive:
                    raise

    def rest_runtime(self, force=False):
        """Confirm torque is off before systemd can terminate the hardware owner."""
        # Re-resolve with retries: a restarting runtime has no usable PID yet.
        client, status = self.wait_runtime_client()
        # A powered-off robot can have been moved by hand since its last rest.
        # Forced deployment rest always measures a fresh goto before release.
        if status.get('torque_enabled') or force:
            self.stop_playback(client)
            if status.get('mode') == 'observe':
                self.run(*client, '--configure', capture_output=True)
            if status.get('mode') in ('observe', 'configured'):
                self.run(*client, '--enable', capture_output=True)
            self.run(*client, '--goto', 'rest', '--duration', '3.0', '--wait', capture_output=True, timeout=30)
            self.run(*client, '--disable', capture_output=True)
        status = json.loads(self.run(*client, '--status', capture_output=True, text=True).stdout)
        if status.get('torque_enabled') is not False:
            raise RuntimeError('Mechanical rest/torque-off was not confirmed; runtime was left running')

    def daemon_command(self, client, command):
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(3)
            connection.connect(client[2])
            connection.sendall(command.encode() + b'\n')
            with connection.makefile('rb') as stream:
                raw = stream.readline(1024 * 1024 + 1)
        if len(raw) > 1024 * 1024:
            raise RuntimeError('Oversized runtime response')
        value = json.loads(raw)
        if value.get('ok') is not True:
            raise RuntimeError(f'Runtime rejected {command}: {value.get("error")}')
        return value

    def settle_runtime(self, client, timeout=30):
        """Use the rest lifecycle so the completed deployment can wake by voice."""
        result = self.daemon_command(client, 'character rest')
        run = result['run_id']
        deadline = time.monotonic() + timeout
        status = lifecycle = {}
        while time.monotonic() < deadline:
            status = json.loads(self.run(*client, '--status', capture_output=True, text=True, timeout=5).stdout)
            last = status.get('last_motion') or {}
            lifecycle = self.daemon_command(client, 'character status')
            rest = lifecycle.get('rest') or {}
            if rest.get('state') == 'fault' or (last.get('run_id') == run and last.get('state') in ('cancelled', 'timed_out')):
                raise RuntimeError(f'Deployment rest run {run} failed: movement={last}, rest={rest}; torque has not been forcibly released')
            if (last.get('run_id') == run and last.get('state') == 'completed' and
                    rest.get('state') == 'resting' and status.get('torque_enabled') is False and
                    rest.get('light_on') is False):
                return
            time.sleep(.1)
        raise RuntimeError(f'Deployment rest run {run} was not confirmed: '
                           f'movement={status.get("last_motion")}, torque_enabled={status.get("torque_enabled")}, '
                           f'rest={lifecycle.get("rest")}; runtime remains available for recovery')

    def smoke_runtime(self, release, timeout=20):
        metadata = json.loads((release / 'release.json').read_text())
        client, status = self.wait_runtime_client(metadata['revision'], timeout)
        print('Confirming the new runtime is at rest before physical smoke playback.', flush=True)
        self.settle_runtime(client)
        failure = None
        try:
            print('Running the physical deployment smoke test: lights, audio and both expressive arcs.', flush=True)
            status = json.loads(self.run(*client, '--status', capture_output=True, text=True, timeout=5).stdout)
            self.stop_playback(client)
            if status.get('mode') == 'observe':
                self.run(*client, '--configure', capture_output=True)
            if status.get('mode') in ('observe', 'configured'):
                self.run(*client, '--enable', capture_output=True)
            # V2's zero_reference is a CAD preview, not a measured pose; home was
            # captured on the hardware and is a short move from rest.
            pose = 'home' if metadata.get('hardware', 'v1') == 'v2' else 'zero_reference'
            print(f'Smoke pose: {pose}', flush=True)
            self.run(*client, '--goto', pose, '--duration', '3.0', '--wait', timeout=30)
            for scene in ('deployment_smoke', 'acknowledge_left', 'acknowledge_right', 'return_home'):
                print(f'Smoke scene: {scene}', flush=True)
                self.run(*client, '--run-scene', scene, '--wait', timeout=60)
        except BaseException as error:
            failure = error
            raise
        finally:
            print('Returning Orion to measured rest, lights off and torque off.', flush=True)
            try:
                if failure is not None:
                    # A failed step may have restarted oriond; reconnect to the
                    # running runtime instead of reusing the old client.
                    client, _ = self.wait_runtime_client(metadata['revision'], timeout)
                self.settle_runtime(client)
            except BaseException as error:
                if failure is not None:
                    raise RuntimeError(f'Physical smoke failed: {failure}; final rest also failed: {error}') from failure
                raise

    def ready(self, release, home, timeout=180):
        metadata = json.loads((release / 'release.json').read_text())
        deadline = time.monotonic() + timeout
        last_error, attempts = None, 0
        while time.monotonic() < deadline:
            try:
                waiting = [name for name in SERVICES if not self.state(name)['active']]
                if waiting:
                    raise RuntimeError(f'Waiting for {", ".join(waiting)} to start')
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
                    str(release / 'voice/models/wake/hey_orion_reference.rpw'))).name
                expected_wake_threshold = float(option(listener_args, '--threshold', ACTIVE_WAKE_THRESHOLD))
                expected_verifier = '--no-verifier' not in listener_args
                if not (status.get('coordinator_running') and not status.get('error') and
                        status.get('project_root') == str(release) and status.get('revision') == metadata['revision'] and
                        ready.get('asr', {}).get('provider') == 'qwen3-asr' and
                        ready.get('tts', {}).get('provider') == expected_provider and
                        ready.get('tts', {}).get('model') == expected_model and
                        ready.get('agent', {}).get('model') == expected_agent and
                        ready.get('agent', {}).get('effort') == expected_effort and
                        ready.get('wake', {}).get('model') == expected_wake_model and
                        ready.get('wake', {}).get('threshold') == expected_wake_threshold and
                        bool((ready.get('wake', {}).get('verifier') or {}).get('active')) == expected_verifier):
                    raise RuntimeError('The requested release is not speech-ready')
                client = [release / 'runtime/target/release/oriond', '--socket', runtime_socket]
                runtime = json.loads(self.run(*client, '--status', capture_output=True, text=True, timeout=5).stdout)
                if runtime.get('build_revision') != metadata['revision'] or runtime.get('hardware', 'v1') != metadata.get('hardware', 'v1'):
                    raise RuntimeError('The old runtime is still active')
                rest = self.daemon_command(client, 'character status').get('rest') or {}
                if (runtime.get('torque_enabled') is not False or
                        rest.get('state') != 'resting' or rest.get('light_on') is not False):
                    raise RuntimeError('The deployed runtime has not remained at rest after the smoke test')
                # The gateway must reach the same authenticated voice host.
                token = token_file.read_text().strip()
                request = urllib.request.Request(f'http://127.0.0.1:{port}/api/v2/voice/request',
                    data=b'{"method":"status"}', headers={'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'})
                with urllib.request.urlopen(request, timeout=3) as response:
                    gateway = json.load(response)
                if gateway.get('pid') != status.get('pid'):
                    raise RuntimeError('Gateway is connected to a different release')
                return
            except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
                last_error = error
                attempts += 1
                if attempts % 30 == 0:  # about every 15 s
                    print(f'Still waiting for readiness: {error}', flush=True)
                time.sleep(.5)
        raise RuntimeError(f'Pi release did not become ready: {last_error}. Check runtime, Qwen, TTS, Codex '
                           f'and gateway logs.{journal_tail("orion-voice-stack")}')


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


def journal_tail(service, lines=20):
    """Recent unit log lines for a failure message; empty when unreadable."""
    try:
        result = subprocess.run(['journalctl', '-u', service, '-n', str(lines), '--no-pager', '-o', 'cat'],
                                capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.SubprocessError):
        return ''
    output = result.stdout.strip()
    return f'\nRecent {service} log:\n{output}' if result.returncode == 0 and output else ''


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
    replacing_hardware = previous and previous.get('hardware', 'v1') != release_hardware(release)
    if replacing_hardware and any(system.state(name)['active'] for name in SERVICES):
        raise RuntimeError('Stop the old hardware services before replacing the lamp; no services were changed')
    folder, data = snapshot(root, [*files, installed], system)
    data['release'] = str(release)
    atomic_json(folder / 'state.json', data)
    atomic_json(pending, {'transaction': str(folder)})
    switched = False
    try:
        # Quiesce companions first. Failure to rest never forces termination of the runtime.
        for name in STOP_ORDER[:-1]:
            system.stop(name)
        if data['services']['oriond']['enabled'] != 'not-found' and not replacing_hardware:
            if not data['services']['oriond']['active']:
                system.start('oriond')
                for name in STOP_ORDER[:-1]:
                    system.stop(name)
            print('Returning the current runtime to rest before updating code and YAML.', flush=True)
            system.rest_runtime(force=True)
        switched = True
        data['status'] = 'switching'; atomic_json(folder / 'state.json', data)
        system.stop('oriond')
        for path, value in files.items():
            if value is None:
                system.remove(path)
            else:
                mode = (path.stat().st_mode & 0o777 if path.exists() else
                        0o644 if str(path).startswith('/etc/') or path.suffix in ('.yaml', '.yml') else 0o600)
                system.write(path, value.encode(), mode)
        system.reload()
        for name in SERVICES:
            if data['services'][name]['enabled'] == 'not-found':
                system.enable(name, 'enabled')
        system.start('oriond')
        # oriond Wants= the listener. Keep all voice/Studio companions quiet
        # until physical smoke playback has settled back to rest.
        for name in STOP_ORDER[:-1]:
            system.stop(name)
        system.smoke_runtime(release)
        for name in SERVICES[1:]:
            system.start(name)
        system.ready(release, home)
        atomic_json(installed, {'version': 2, 'release': str(release), 'hardware': release_hardware(release), 'rollback': str(folder)})
        data['status'] = 'committed'; atomic_json(folder / 'state.json', data)
    except BaseException as activation_error:
        data['activation_error'] = failure_details(activation_error)
        atomic_json(folder / 'state.json', data)
        if switched:
            try:
                restore(folder, data, system)
            except BaseException as recovery_error:
                data['recovery_error'] = failure_details(recovery_error)
                data['status'] = 'rollback_failed'; atomic_json(folder / 'state.json', data)
                raise RuntimeError(
                    f'Activation failed: {data["activation_error"]}\n'
                    f'Recovery failed: {data["recovery_error"]}\n'
                    f'Recovery could not finish; preserve {folder} and run --rollback'
                ) from activation_error
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
    prune_releases(release, previous.get('release') if previous else None)
    # Retain exactly one transaction owned by this installer.
    for old in folder.parent.iterdir():
        if old.is_dir() and not old.is_symlink() and re.fullmatch(r'[0-9a-f]{32}', old.name) and old != folder:
            try:
                shutil.rmtree(old)
            except OSError as error:
                print(f'Release is active; could not remove old rollback {old}: {error}')
    return folder


def prune_releases(active, rollback_release):
    """Keep the active release and the one rollback restores; delete older ones."""
    keep = {Path(active).resolve()}
    if rollback_release:
        keep.add(Path(rollback_release).resolve())
    for old in Path(active).resolve().parent.iterdir():
        if old.is_dir() and not old.is_symlink() and re.fullmatch(r'[0-9a-f]{12}-[0-9a-f]{8}', old.name) \
                and old.resolve() not in keep:
            try:
                shutil.rmtree(old)
                print(f'Removed old release {old.name}')
            except OSError as error:
                print(f'Could not remove old release {old}: {error}')


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
    hardware = release_hardware(release)
    calibration = calibration_path(home, hardware)
    if not calibration.is_file():
        raise RuntimeError(f'Missing {hardware} servo calibration: {calibration}')
    token = home / '.config/orion/studio-token'
    if not token.is_file():
        raise RuntimeError(f'Missing Studio pairing token: {token}. Create it with: '
                           f'python3 {release / "orion_studio/gateway.py"} create-token --token-file {token}')
    check_calibration(calibration, hardware)
    stray = system.foreign_runtimes()
    if stray:
        raise RuntimeError(f'Stop the oriond started by hand (PID {", ".join(stray)}) before deploying; '
                           'it holds the servo port and the runtime socket')
    environment = {**os.environ, **read_env(files[home / '.config/orion/voice-stack.env']),
                   'ORION_PROJECT_ROOT': str(release), 'ORION_ONBOARD': '1', 'ORION_SPEECH_BACKEND': 'pi'}
    system.run(environment['ORION_STUDIO_CODEX_BIN'], 'login', 'status')
    system.run(release / 'orion-service/target/release/orion-service', 'check', env=environment)


def check_calibration(path, hardware):
    """Refuse a calibration the runtime would reject, before any service stops."""
    try:
        joints = json.loads(path.read_text()).get('joints', {})
    except (ValueError, AttributeError) as error:
        raise RuntimeError(f'Calibration {path} is not readable JSON: {error}') from None
    expected = profile(hardware)['joints']
    for name, joint in joints.items():
        if name in expected and joint.get('servo_id') != expected[name]['servo_id']:
            raise RuntimeError(f'{path} maps {name} to servo {joint.get("servo_id")}, but the {hardware} '
                               f'profile uses servo {expected[name]["servo_id"]}')
        low = joint['neutral_raw'] + joint['safe_min_delta_raw']
        high = joint['neutral_raw'] + joint['safe_max_delta_raw']
        if low < 0 or high > 4095:
            raise RuntimeError(f'{name} safe range covers raw {low}..{high}, crossing 0/4095; '
                               'run orion-centre-servos before deploying')


def validate_catalog(release, project, calibration, system, assets, home=None):
    # Compile the planned catalog before changing live files or stopping services.
    with tempfile.TemporaryDirectory(prefix='orion-catalog-') as temporary:
        preview = Path(temporary)
        shutil.copytree(project / 'motion', preview / 'motion', symlinks=True)
        for path, value in assets.items():
            if path.is_relative_to(project) and path.relative_to(project).parts[0] == 'motion':
                target = preview / path.relative_to(project)
                if value is None:
                    target.unlink(missing_ok=True)
                else:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_text(value)
        hardware = release_hardware(release)
        selected = profile(hardware)
        poses = preview / selected['poses']
        motions = preview / selected['motions']
        if hardware == 'v2':
            names = sorted(p.stem for p in motions.rglob('*.yaml'))
        else:
            names = ('look_at_left_expressive', 'look_at_right_expressive')
        for motion in names:
            system.run(release / 'runtime/target/release/orion-trajectory',
                '--hardware', hardware, '--motion', motion, '--start-pose', 'attentive',
                '--pose-file', poses,
                '--motions-directory', motions, '--calibration', calibration,
                stdout=subprocess.DEVNULL)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path.home() / '.local/share/orion/voice-stack')
    parser.add_argument('--release', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--runtime-project', type=Path, default=Path.home() / 'dev/orion',
                        help='Live catalog; built-in YAML updates, user assets and calibration are preserved')
    parser.add_argument('--rollback', action='store_true')
    parser.add_argument('--plan', action='store_true', help='Show affected paths without modifying files or services')
    args = parser.parse_args()
    root, release, home = args.root.resolve(), args.release.resolve(), Path.home()
    if args.plan:
        files = render_plan(release, root, args.runtime_project.resolve(), home, pwd.getpwuid(os.getuid()).pw_name)
        files.update(catalog_plan(release, args.runtime_project.resolve(), root))
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
        assets = catalog_plan(release, args.runtime_project.resolve(), root)
        files.update(assets)
        preflight(release, home, files, system)
        hardware = release_hardware(release)
        # Check boot persistence before any motion or service switch.
        system.run(release / 'hardware/lighting/verify-persistent.sh')
        if hardware == 'v2':
            system.run('arecord', '-l')
            system.run('aplay', '-l')
        validate_catalog(release, args.runtime_project.resolve(), calibration_path(home, hardware), system, assets, home)
        def interrupted(signum, frame):
            raise KeyboardInterrupt('Deployment interrupted')
        signal.signal(signal.SIGTERM, interrupted)
        folder = activate(root, release, home, files, system)
        metadata = json.loads((release / 'release.json').read_text())
        states = ', '.join(f'{name} {"active" if system.state(name)["active"] else "inactive"}' for name in SERVICES)
        print(f'Complete Pi release is ready; physical smoke test passed and Orion is at rest with lights and torque off.\n'
              f'  revision {metadata["revision"]} for {hardware}\n  services: {states}\n'
              f'  Studio gateway: http://{socket.gethostname()}.local:7447 '
              f'(token: ~/.config/orion/studio-token)\n  rollback snapshot: {folder}')


if __name__ == '__main__':
    main()
