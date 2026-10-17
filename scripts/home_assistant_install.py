"""Install the independent Home Assistant container on Raspberry Pi OS."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import pwd
import re
import secrets
import shutil
import socket
import subprocess
import tempfile

NAME = 'orion-home-assistant'


def execute(*args, **kwargs):
    kwargs.setdefault('check', True)
    return subprocess.run([str(arg) for arg in args], **kwargs)


def image_pin(manifest):
    pin = json.loads(Path(manifest).read_text())
    if (pin.get('image') != 'ghcr.io/home-assistant/home-assistant'
        or not re.fullmatch(r'\d{4}\.\d+\.\d+', pin.get('version', ''))
        or not re.fullmatch(r'sha256:[a-f0-9]{64}', pin.get('digest', ''))
        or pin.get('platform') != 'linux/arm64'):
        raise RuntimeError('Invalid Home Assistant release manifest')
    return pin, f"{pin['image']}:{pin['version']}@{pin['digest']}"


def save_metadata(data, state, uid, gid):
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode='w', dir=data, prefix='.home-assistant-', delete=False) as output:
            temporary = Path(output.name)
            json.dump(state, output); output.flush(); os.fsync(output.fileno())
            os.fchown(output.fileno(), uid, gid)
        temporary.replace(data / 'home-assistant-install.json')
    finally:
        if temporary:
            temporary.unlink(missing_ok=True)


def install(home, uid, gid, manifest, update=False, run=execute, which=shutil.which):
    pin, image = image_pin(manifest)
    data = Path(home) / '.local/share/orion'
    data.mkdir(parents=True, exist_ok=True)
    os.chown(data, uid, gid)
    lock = data / '.home-assistant-install.lock'
    with lock.open('a') as guard:
        os.chown(lock, uid, gid)
        fcntl.flock(guard, fcntl.LOCK_EX)
        missing = not which('docker')
        if missing:
            run('apt-get', 'update')
            run('env', 'DEBIAN_FRONTEND=noninteractive', 'apt-get', 'install', '-y', '--no-install-recommends', 'docker.io')
            run('systemctl', 'enable', '--now', 'docker.service')
        inspected = run('docker', 'container', 'inspect', NAME, capture_output=True, text=True, check=False)
        if not missing and inspected.returncode != 0:
            run('systemctl', 'enable', '--now', 'docker.service')
            inspected = run('docker', 'container', 'inspect', NAME, capture_output=True, text=True, check=False)
        previous = json.loads(inspected.stdout)[0] if inspected.returncode == 0 else None
        if previous and (not update or previous['Config']['Image'] == image):
            if not (data / 'home-assistant-install.json').exists() and re.fullmatch(r'[a-f0-9]{64}', previous.get('Id', '')):
                reference = previous['Config']['Image']
                version = re.search(r':(\d{4}\.\d+\.\d+)(?:@|$)', reference)
                save_metadata(data, {'container_id': previous['Id'], 'version': version.group(1) if version else None, 'image': reference}, uid, gid)
            if not previous['State']['Running']:
                run('docker', 'start', NAME)
            print(f'Home Assistant is running at http://{socket.gethostname()}:8123', flush=True)
            return {'installed': True, 'changed': not previous['State']['Running']}
        # Pull before stopping an existing container so download failures leave it running.
        run('docker', 'pull', '--platform', pin['platform'], image)
        config = data / 'home-assistant'
        config.mkdir(exist_ok=True)
        os.chown(config, uid, gid)
        previous_name = NAME + '-previous-' + secrets.token_hex(4)
        if previous:
            run('docker', 'rename', NAME, previous_name)
        try:
            if previous:
                run('docker', 'stop', previous_name)
            result = run('docker', 'run', '-d', '--name', NAME, '--network=host', '--restart', 'unless-stopped',
                '--memory', '1g', '--cpu-shares', '256', '--volume', f'{config}:/config',
                '--volume', '/run/dbus:/run/dbus:ro', '--env', 'TZ=' + timezone(),
                '--platform', pin['platform'], image, capture_output=True, text=True)
        except (OSError, subprocess.SubprocessError):
            if previous:
                run('docker', 'rm', '-f', NAME, check=False)
                run('docker', 'rename', previous_name, NAME)
                if previous['State']['Running']:
                    run('docker', 'start', NAME)
            raise
        state = {'container_id': result.stdout.strip(), 'version': pin['version'], 'image': image}
        save_metadata(data, state, uid, gid)
        if previous:
            run('docker', 'rm', previous_name)
        print(f'Home Assistant is running at http://{socket.gethostname()}:8123', flush=True)
        return {'installed': True, 'changed': True}


def timezone():
    from zoneinfo import ZoneInfo, ZoneInfoNotFoundError
    candidates = [os.environ.get('TZ', '')]
    try:
        candidates.append(Path('/etc/timezone').read_text().strip())
    except OSError:
        pass
    try:
        candidates.append(str(Path('/etc/localtime').resolve()).split('/zoneinfo/', 1)[1])
    except (OSError, IndexError):
        pass
    for value in candidates:
        try:
            ZoneInfo(value)
            return value
        except (ValueError, ZoneInfoNotFoundError):
            continue
    return 'UTC'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--user', required=True)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--update', action='store_true')
    args = parser.parse_args()
    if os.geteuid() != 0:
        parser.error('This helper runs as root through the Orion installer service.')
    user = pwd.getpwnam(args.user)
    install(Path(user.pw_dir), user.pw_uid, user.pw_gid, args.manifest, args.update)


if __name__ == '__main__':
    main()
