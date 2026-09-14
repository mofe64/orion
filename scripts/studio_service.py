"""Build immutable Studio releases and manage the macOS user LaunchAgent."""
import argparse
import contextlib
import fcntl
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
PACKAGES = {'studio-service', 'coordinator', 'agent', 'speech'}


def run(arguments, **kwargs):
    return subprocess.run([str(arg) for arg in arguments], check=True, **kwargs)


class Service:
    def __init__(self):
        self.home = Path(os.environ.get('ORION_STUDIO_SERVICE_HOME',
                                       str(Path.home() / '.local/share/orion/studio-service'))).expanduser().resolve()
        self.label = os.environ.get('ORION_STUDIO_LAUNCHD_LABEL', 'org.orion.studio.headless')
        if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9.-]+', self.label):
            raise ValueError('Invalid launchd label')
        self.domain = f'gui/{os.getuid()}'
        self.target = f'{self.domain}/{self.label}'
        agents = Path(os.environ.get('ORION_STUDIO_LAUNCH_AGENTS', str(Path.home() / 'Library/LaunchAgents')))
        self.plist = agents / f'{self.label}.plist'
        self.current = self.home / 'current'

    @contextlib.contextmanager
    def transaction(self):
        self.home.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.home.chmod(0o700)
        with (self.home / 'update.lock').open('a') as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise RuntimeError('Another Studio installation or update is running') from None
            yield

    def loaded(self):
        return subprocess.run(['launchctl', 'print', self.target],
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0

    def stop(self):
        if self.loaded():
            run(['launchctl', 'bootout', self.target])
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            with (self.home / 'owner.lock').open('a') as lock:
                try:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    return
                except BlockingIOError:
                    time.sleep(.1)
        raise RuntimeError('Studio still owns voice. Close its embedded UI session and retry.')

    def start(self):
        if not self.plist.exists() or not self.current.is_dir():
            raise RuntimeError('Install first: scripts/studio-headless.sh install')
        run(['launchctl', 'enable', self.target])
        if not self.loaded():
            run(['launchctl', 'bootstrap', self.domain, self.plist])
        run(['launchctl', 'kickstart', self.target])
        return self.wait_ready()

    def status(self):
        executable = self.current / 'bin/orion-studio-headless'
        if not executable.is_file():
            return None
        try:
            result = subprocess.run([str(executable), 'status'],
                                    env={**os.environ, 'ORION_STUDIO_SERVICE_HOME': str(self.home)},
                                    text=True, capture_output=True, timeout=8)
        except subprocess.TimeoutExpired:
            return None
        if result.returncode:
            return None
        return json.loads(result.stdout)

    def wait_ready(self):
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            status = self.status()
            if status is not None:
                return status
            time.sleep(.2)
        raise RuntimeError(f'Studio did not start; inspect {self.home / "logs/stderr.log"}')

    def write_plist(self, revision):
        self.plist.parent.mkdir(parents=True, exist_ok=True)
        logs = self.home / 'logs'; logs.mkdir(exist_ok=True)
        environment = {
            'HOME': str(Path.home()),
            'PATH': os.environ.get('PATH', '/usr/bin:/bin:/usr/sbin:/sbin'),
            'ORION_PROJECT_ROOT': str(self.current),
            'ORION_STUDIO_SERVICE_HOME': str(self.home),
            'ORION_RELEASE_REVISION': revision,
        }
        for name in ('ORION_STUDIO_CODEX_BIN', 'ORION_SOUL_PATH', 'ORION_MEMORY_PATH',
                     'ORION_PI_VOICE_URL', 'HF_HOME', 'HF_HUB_CACHE', 'HUGGINGFACE_HUB_CACHE',
                     'XDG_CACHE_HOME', 'ORION_STUDIO_ASR_MODEL', 'ORION_STUDIO_TTS_MODEL'):
            if name in os.environ:
                environment[name] = os.environ[name]
        data = {
            'Label': self.label,
            'ProgramArguments': [str(self.current / 'bin/orion-studio-headless'), 'serve'],
            'WorkingDirectory': str(self.current),
            'EnvironmentVariables': environment,
            'RunAtLoad': True, 'KeepAlive': True, 'ThrottleInterval': 10,
            'ExitTimeOut': 20, 'ProcessType': 'Interactive',
            'StandardOutPath': str(logs / 'stdout.log'),
            'StandardErrorPath': str(logs / 'stderr.log'),
        }
        temporary = self.plist.with_suffix('.tmp')
        temporary.write_bytes(plistlib.dumps(data)); temporary.chmod(0o600)
        temporary.replace(self.plist)
        run(['plutil', '-lint', self.plist])

    def switch(self, release):
        temporary = self.home / f'current-{uuid.uuid4().hex}'
        temporary.symlink_to(release, target_is_directory=True)
        temporary.replace(self.current)

    def prepare(self, pull):
        if pull:
            dirty = run(['git', 'status', '--porcelain'], cwd=ROOT, capture_output=True, text=True).stdout
            if dirty.strip():
                raise RuntimeError('--pull requires a clean checkout; commit your changes first')
            run(['git', 'fetch'], cwd=ROOT)
            run(['git', 'merge', '--ff-only', '@{upstream}'], cwd=ROOT)
        revision = run(['git', 'rev-parse', '--short=12', 'HEAD'], cwd=ROOT, capture_output=True, text=True).stdout.strip()
        dirty = run(['git', 'status', '--porcelain'], cwd=ROOT, capture_output=True, text=True).stdout.strip()
        if dirty:
            revision += '-working'
        release = self.home / 'releases' / f'{revision}-{uuid.uuid4().hex[:8]}'
        release.mkdir(parents=True)
        try:
            listed = run(['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'],
                         cwd=ROOT, capture_output=True).stdout.split(b'\0')
            for raw in set(listed):
                if not raw:
                    continue
                relative = Path(os.fsdecode(raw))
                source = ROOT / relative
                if relative.parts[0] in PACKAGES and source.is_file():
                    destination = release / relative
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(source, destination)
            env = {**os.environ, 'CARGO_TARGET_DIR': str(self.home / 'build'), 'ORION_PROJECT_ROOT': str(release),
                   'ORION_STUDIO_VOICE_PYTHON': str(release / 'speech/.venv/bin/python')}
            # Virtual environments contain absolute paths, so build them at their final location.
            run(['uv', 'sync', '--project', release / 'speech', '--python', '3.12', '--locked'], env=env)
            for package in ('agent', 'coordinator', 'studio-service'):
                run(['cargo', 'test', '--manifest-path', release / package / 'Cargo.toml', '--all-targets', '--locked'], env=env)
            run([release / 'speech/.venv/bin/python', '-m', 'unittest', 'discover', '-s', release / 'speech/tests', '-v'], cwd=release / 'speech', env=env)
            run(['cargo', 'build', '--manifest-path', release / 'studio-service/Cargo.toml', '--release', '--locked'], env=env)
            binary = release / 'bin/orion-studio-headless'; binary.parent.mkdir()
            shutil.copy2(self.home / 'build/release/orion-studio-headless', binary)
            run([binary, 'check'], env=env)
            (release / 'release.json').write_text(json.dumps({'revision': revision, 'source': str(ROOT)}, indent=2))
            return release, revision
        except BaseException:
            shutil.rmtree(release)
            raise

    def update(self, pull=False):
        release, revision = self.prepare(pull)
        previous = self.current.resolve() if self.current.is_symlink() else None
        old_plist = self.plist.read_bytes() if self.plist.exists() else None
        installed = (self.home / 'installed').exists()
        was_loaded = self.loaded()
        switched = False
        try:
            self.stop()
            self.switch(release); switched = True
            self.write_plist(revision)
            (self.home / 'installed').touch(mode=0o600)
            status = self.start()
            if status['revision'] != revision:
                raise RuntimeError('The running service did not load the selected release')
        except BaseException:
            if switched:
                self.stop()
                if previous is not None:
                    self.switch(previous)
                else:
                    self.current.unlink(missing_ok=True)
                if old_plist is not None:
                    self.plist.write_bytes(old_plist)
                else:
                    self.plist.unlink(missing_ok=True)
                if not installed:
                    (self.home / 'installed').unlink(missing_ok=True)
                if previous is not None and was_loaded:
                    self.start()
            raise
        print(f'Studio headless updated to {revision}; starts automatically after login.')
        if not status['coordinator_running']:
            print('Control service is ready. Voice is starting or awaiting pairing; use status for details.')
        return release


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='mode', required=True)
    manage = commands.add_parser('manage')
    manage.add_argument('action', choices=['install', 'start', 'stop', 'restart', 'status', 'logs', 'uninstall'])
    update = commands.add_parser('update'); update.add_argument('--pull', action='store_true')
    args = parser.parse_args()
    if sys.platform != 'darwin':
        parser.error('The launchd scripts require macOS. The headless binary can run directly for development.')
    service = Service()
    if args.mode == 'manage' and args.action == 'status':
        status = service.status()
        print(json.dumps(status, indent=2) if status else 'Studio headless is stopped.')
        return 0 if status else 1
    if args.mode == 'manage' and args.action == 'logs':
        run(['tail', '-n', '100', '-F', service.home / 'logs/stderr.log']); return 0
    with service.transaction():
        if args.mode == 'update' or args.action == 'install':
            service.update(getattr(args, 'pull', False))
        elif args.action == 'start':
            print(json.dumps(service.start(), indent=2))
        elif args.action == 'restart':
            service.stop(); print(json.dumps(service.start(), indent=2))
        elif args.action == 'stop':
            service.stop(); print('Stopped. Automatic startup resumes at the next login.')
        elif args.action == 'uninstall':
            service.stop()
            service.plist.unlink(missing_ok=True)
            (service.home / 'installed').unlink(missing_ok=True)
            (service.home / 'connection.json').unlink(missing_ok=True)
            print('Automatic startup removed. Releases and user data are retained.')
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
        print(f'Studio service: {error}', file=sys.stderr)
        sys.exit(1)
