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
from avahi_config import configure_avahi

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


# Home Assistant images use zstd layers, which need Docker Engine 23 or later.
# Debian Bookworm's docker.io is 20.10, so Orion installs Docker's own packages.
DOCKER_MINIMUM = 23
DOCKER_KEY_URL = 'https://download.docker.com/linux/debian/gpg'
DOCKER_KEY_FINGERPRINT = '9DC858229FC7DD38854AE2D88D81803C0EBFCD88'
DOCKER_KEYRING = Path('/etc/apt/keyrings/docker.asc')
DOCKER_SOURCES = Path('/etc/apt/sources.list.d/docker.list')
OS_RELEASE = Path('/etc/os-release')


def engine_version(run):
    result = run('docker', 'version', '--format', '{{.Server.Version}}', capture_output=True, text=True, check=False)
    match = re.match(r'(\d+)\.', result.stdout.strip()) if result.returncode == 0 else None
    return int(match.group(1)) if match else None


def install_docker(run):
    codename = re.search(r'^VERSION_CODENAME=(\w+)', OS_RELEASE.read_text(), re.M).group(1)
    run('apt-get', 'update')
    run('env', 'DEBIAN_FRONTEND=noninteractive', 'apt-get', 'install', '-y', '--no-install-recommends', 'ca-certificates', 'curl', 'gnupg')
    key = run('curl', '-fsSL', DOCKER_KEY_URL, capture_output=True).stdout
    listing = run('gpg', '--show-keys', '--with-colons', input=key, capture_output=True).stdout
    if DOCKER_KEY_FINGERPRINT.encode() not in listing:
        raise RuntimeError('Docker signing key fingerprint does not match')
    DOCKER_KEYRING.parent.mkdir(parents=True, exist_ok=True)
    DOCKER_KEYRING.write_bytes(key)
    DOCKER_KEYRING.chmod(0o644)
    DOCKER_SOURCES.write_text(f'deb [arch=arm64 signed-by={DOCKER_KEYRING}] https://download.docker.com/linux/debian {codename} stable\n')
    run('apt-get', 'remove', '-y', 'docker.io', 'docker-doc', 'podman-docker', 'containerd', 'runc', check=False)
    run('apt-get', 'update')
    run('env', 'DEBIAN_FRONTEND=noninteractive', 'apt-get', 'install', '-y', '--no-install-recommends',
        'docker-ce', 'docker-ce-cli', 'containerd.io')
    run('systemctl', 'enable', '--now', 'docker.service')


def ensure_docker(run, which):
    """Leave a running Docker Engine that can unpack Home Assistant's image."""
    version = None
    if which('docker'):
        version = engine_version(run)
        if version is None:
            run('systemctl', 'enable', '--now', 'docker.service', check=False)
            version = engine_version(run)
    if version is None or version < DOCKER_MINIMUM:
        install_docker(run)


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
        ensure_docker(run, which)
        configure_avahi(run)
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
