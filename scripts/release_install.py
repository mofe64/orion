"""Install verified release files with offline venvs and the existing rollback transaction."""
import argparse
import json
import os
from pathlib import Path
import pwd
import shutil
import subprocess
import sys
import tempfile

from install_pi_voice_stack import System, activate, atomic_json, deployment_lock, rollback, snapshot, restore
from native_packages import UNAVAILABLE, pinned_packages
from pi_catalog import catalog_plan
from pi_release_bundle import install_wheels, verify_payload
from pi_release_platform import check_platform, read_platform
from pi_service_config import MANAGED_SERVICES, render_plan
from prepare_pi_voice_stack import CODEX_VERSION, prepare_assets
from release_models import prepare_vision_models
from release_assets import prepare_catalog

COMPONENTS = ('speech', 'voice', 'vision', 'motion', 'orion_studio')
PYTHON = Path('/opt/orion/python/bin/python3.12')


def run(*args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def prepare_python(source):
    destination = PYTHON.parent.parent
    if destination.exists():
        if not PYTHON.is_file():
            raise RuntimeError(f'Unexpected Python directory: {destination}; preserve it before retrying')
        found = run(PYTHON, '-c', 'import platform; print(platform.python_version())', capture_output=True, text=True).stdout.strip()
        if found != '3.12.13': raise RuntimeError(f'Installed managed Python is {found}; expected 3.12.13')
        return
    run('sudo', 'install', '-d', destination.parent)
    temporary = Path(run('sudo', 'mktemp', '-d', str(destination) + '.XXXXXXXX', capture_output=True, text=True).stdout.strip())
    try:
        run('sudo', 'cp', '-a', str(source) + '/.', str(temporary) + '/')
        run('sudo', 'chmod', '0755', temporary)
        run(temporary / 'bin/python3.12', '-c', 'import ssl, sqlite3, venv; import sys; assert sys.version_info[:3] == (3, 12, 13)')
        run('sudo', 'mv', '-T', temporary, destination)
    finally:
        run('sudo', 'rm', '-rf', temporary)


def has_hailo():
    return '1e60:' in run('lspci', '-Dn', capture_output=True, text=True).stdout.lower()


def system_packages(codename, hailo, pins):
    common = ['alsa-utils', 'libdbus-1-3', 'libusb-1.0-0', 'libcap2', 'libgomp1', 'libarchive13' if codename == 'bookworm' else 'libarchive13t64',
              'raspi-utils', 'dkms', 'device-tree-compiler', 'linux-image-rpi-2712', 'linux-headers-rpi-2712',
              'libssl3' if codename == 'bookworm' else 'libssl3t64']
    return common + pinned_packages(pins, hailo)


def install_system_packages(root, codename, hailo, pins):
    packages = pinned_packages(pins, hailo)
    names = [package.split('=', 1)[0] for package in packages]
    run('sudo', 'apt-get', 'update')
    available = run('apt-cache', 'madison', *names, capture_output=True, text=True).stdout
    versions = set()
    for line in available.splitlines():
        fields = [field.strip() for field in line.split('|')]
        if len(fields) >= 2:
            versions.add(fields[0].split(':', 1)[0] + '=' + fields[1])
    if set(packages) - versions:
        raise RuntimeError(UNAVAILABLE)
    run('sudo', 'apt-get', 'install', '-y', '--no-install-recommends',
        '--allow-downgrades', '--allow-change-held-packages', *system_packages(codename, hailo, pins))
    state = root / 'native-packages.json'
    previous = json.loads(state.read_text())['packages'] if state.exists() else []
    retired = sorted(set(previous) - set(names))
    if retired:
        run('sudo', 'apt-mark', 'unhold', *retired)
    run('sudo', 'apt-mark', 'hold', *names)
    atomic_json(state, {'packages': names})


def restore_native_packages(root, codename, folder, data):
    if data.get('assets_only'):
        return
    for entry in data['files']:
        if entry['path'] == str(root / 'installation.json') and entry['backup']:
            previous = Path(json.loads(Path(entry['backup']).read_text())['release'])
            manifest = previous / 'scripts/release-native.json'
            if manifest.exists():
                install_system_packages(root, codename, has_hailo(), json.loads(manifest.read_text())[codename])
            return


def check_installation(root):
    release = Path(json.loads((root / 'installation.json').read_text())['release'])
    commands = [
        ('Servo positions', (release / 'runtime/target/release/oriond', '--check')),
        ('Orion services', ('systemctl', '--no-pager', '--full', 'status', *MANAGED_SERVICES)),
        ('AI HAT+', (release / 'vision/.venv/bin/python', release / 'scripts/check_hailo.py', '--root', root)),
    ]
    failed = []
    for label, command in commands:
        print(f'--- {label} ---', flush=True)
        try:
            run(*command)
        except (OSError, subprocess.CalledProcessError) as error:
            print(f'{label}: {error}', flush=True)
            failed.append(label)
    if failed:
        raise RuntimeError('Checks failed: ' + ', '.join(failed))


def configure_hailo(present, boot=Path('/boot/firmware/config.txt')):
    if not present:
        print('AI HAT+: absent; skipping Hailo setup.')
        return
    text = boot.read_text()
    section = 'all'
    last_setting = None
    for line in text.splitlines():
        line = line.partition('#')[0].strip()
        if line.startswith('[') and line.endswith(']'): section = line[1:-1]
        elif line.startswith('dtparam=pciex1_gen='): last_setting = (section, line)
    if last_setting != ('all', 'dtparam=pciex1_gen=3'):
        # A trailing [all] makes this independent of Imager's final conditional.
        run('sudo', 'tee', '-a', boot, input='\n[all]\n# Orion AI HAT+\ndtparam=pciex1_gen=3\n', text=True, stdout=subprocess.DEVNULL)
    print('AI HAT+: PCIe Gen 3 configured; reboot required for driver/firmware changes.')


def prepare_environments(release, wheels, uv, codename, hailo):
    for component in COMPONENTS:
        environment = release / component / '.venv'
        run(uv, 'venv', '--no-config', '--offline', '--no-cache', '--no-python-downloads', '--python', PYTHON, environment)
        # uv's wheel installer is invoked through this bundle's binary, never a
        # system interpreter or inherited pip configuration.
        def offline(*args, **kwargs):
            args = (uv, *args[1:]) if args[0] == 'uv' else args
            return run(*args, **kwargs)
        install_wheels(environment / 'bin/python', wheels / component, offline)
        if component == 'vision':
            packages = sorted((wheels / 'native' / codename).glob('*.whl'))
            if not hailo:
                packages = [p for p in packages if not p.name.startswith('hailort-')]
            run(uv, 'pip', 'install', '--python', environment / 'bin/python', '--offline', '--no-index', '--no-build',
                '--no-deps', '--no-cache', *packages)
            run(environment / 'bin/python', '-c', 'import libcamera, pykms, picamera2')


def install(payload, root, codename):
    manifest = json.loads((payload / 'bundle.json').read_text())
    verify_payload(payload, manifest)
    check_platform(manifest['platform'], read_platform())
    metadata = json.loads((payload / 'release/release.json').read_text())
    pins = json.loads((payload / 'release/scripts/release-native.json').read_text())[codename]
    present = has_hailo()
    release = root / 'releases' / metadata['release_id']
    with deployment_lock(root):
        if (root / 'pending-installation.json').exists():
            raise RuntimeError('An interrupted deployment needs recovery; run this installer with --rollback')
        install_system_packages(root, codename, present, pins)
        configure_hailo(present)
        installed = root / 'installation.json'
        active = json.loads(installed.read_text()) if installed.exists() else {}
        if active.get('release') == str(release):
            print(f'Orion {metadata["version"]} is already installed. Run --check, or finish servo commissioning and start the units.')
            return
        if release.exists():
            if (release / '.orion-preparing').is_file() and (release / '.orion-preparing').read_text() == metadata['release_id']:
                shutil.rmtree(release)
            else:
                raise RuntimeError(f'Unmanaged release directory exists: {release}; preserve it before retrying.')
        release.parent.mkdir(parents=True, exist_ok=True)
        release.mkdir()
        (release / '.orion-preparing').write_text(metadata['release_id'])
        shutil.copytree(payload / 'release', release, dirs_exist_ok=True)
        # The interpreter is portable. Copy its verified distribution verbatim;
        # venvs below are created at their final paths and use no OS site-packages.
        prepare_python(payload / 'python')
        uv = payload / 'bin/uv'
        prepare_environments(release, payload / 'wheels', uv, codename, present)
        prepare_assets(root)
        prepare_vision_models(root, release)
        # Kernel modules depend on the Pi's running kernel, so DKMS handles
        # lighting on the device. Its pinned source ships in the release.
        run('sudo', release / 'hardware/lighting/install-persistent.sh', release / 'native/rpi_ws281x')
        user = pwd.getpwuid(os.getuid()).pw_name
        run('sudo', 'usermod', '-aG', 'dialout,video,gpio,plugdev', user)
        run('sudo', 'install', '-m', '0644', release / 'voice/99-respeaker-xvf3800.rules', '/etc/udev/rules.d/99-respeaker-xvf3800.rules')
        home = Path.home()
        token = home / '.config/orion/studio-token'
        if not token.exists():
            run(release / 'orion_studio/.venv/bin/python', release / 'orion_studio/gateway.py', 'create-token', '--token-file', token)
        inventory = root / 'catalog-assets.json'
        project = Path(json.loads(inventory.read_text())['project']) if inventory.exists() else root / 'catalog'
        project.mkdir(parents=True, exist_ok=True)
        for name in ('motion', 'scenes'):
            if not (project / name).exists():
                shutil.copytree(release / name, project / name, ignore=shutil.ignore_patterns('.venv', '__pycache__', '.pytest_cache'))
        files = render_plan(release, root, project, home, user)
        with tempfile.TemporaryDirectory(prefix='orion-assets-') as temporary:
            source = prepare_catalog(release, Path(temporary) / 'catalog', first_install=not active)
            files.update(catalog_plan(source, project, root))
        files[home / '.config/orion/servo_calibration-v2.json'] = None
        files[Path('/usr/local/bin/oriond')] = '#!/bin/sh\ncd ' + str(project) + '\nexec ' + str(release / 'runtime/target/release/oriond') + ' "$@"\n'
        files[Path('/usr/local/bin/orion-check-hailo')] = '#!/bin/sh\nexec ' + str(release / 'vision/.venv/bin/python') + ' ' + str(release / 'scripts/check_hailo.py') + ' --root ' + str(root) + ' "$@"\n'
        files[Path('/usr/local/bin/codex')] = '#!/bin/sh\nexec ' + str(root / f'codex-{CODEX_VERSION}/bin/codex') + ' "$@"\n'
        activate(root, release, home, files, System(), commissioning=True,
                 before_restore=lambda folder, data: restore_native_packages(root, codename, folder, data))
        (release / '.orion-preparing').unlink()
    print('Release installed. Reboot for lighting, Hailo and group permissions. Run codex login --device-auth, fold into rest, then oriond servo zero and oriond --check.')
    print('Start Orion: sudo systemctl start oriond orion-listener orion-studio-gateway orion-voice-stack orion-vision')



def apply_assets(root, source, system, expected_release=None):
    """Reuse catalog planning, snapshots and recovery without activating app code."""
    with deployment_lock(root):
        if (root / 'pending-installation.json').exists():
            raise RuntimeError('An interrupted deployment needs recovery; run --rollback')
        installed = root / 'installation.json'
        previous = json.loads(installed.read_text())
        if expected_release is not None and previous['release'] != str(expected_release):
            raise RuntimeError('The active app changed during the download; retry the asset update')
        inventory = json.loads((root / 'catalog-assets.json').read_text())
        project = Path(inventory['project'])
        files = catalog_plan(source, project, root)
        folder, data = snapshot(root, [*files, installed], system)
        data['assets_only'] = True
        data['release'] = previous['release']
        atomic_json(folder / 'state.json', data)
        pending = root / 'pending-installation.json'
        atomic_json(pending, {'transaction': str(folder)})
        switched = False
        try:
            running = data['services']['oriond']['active']
            if running:
                from pi_service_config import MANAGED_STOP_ORDER
                for name in MANAGED_STOP_ORDER[:-1]: system.stop(name)
                system.rest_runtime(force=True)
                switched = True
                system.stop('oriond')
            else:
                switched = True
            data['status'] = 'switching'; atomic_json(folder / 'state.json', data)
            for path, value in files.items():
                if value is None: system.remove(path)
                else: system.write(path, value if isinstance(value, bytes) else value.encode(), 0o644)
            if running:
                for name, state in data['services'].items():
                    if state['active']: system.start(name)
            if running:
                for name, state in data['services'].items():
                    if not state['active'] and system.state(name)['active']: system.stop(name)
            previous['rollback'] = str(folder)
            atomic_json(installed, previous)
            data['status'] = 'committed'; atomic_json(folder / 'state.json', data)
        except BaseException:
            if switched:
                restore(folder, data, system)
            else:
                # Rest failed before any catalog change. Keep the existing
                # daemon and restore only companions stopped for the attempt.
                for name, state in data['services'].items():
                    if name != 'oriond' and state['active']: system.start(name)
                data['status'] = 'rolled_back'; atomic_json(folder / 'state.json', data)
            pending.unlink(missing_ok=True)
            raise
        pending.unlink()
        return folder


def install_assets(root, version=None):
    installed = json.loads((root / 'installation.json').read_text())
    app = Path(installed['release'])
    with tempfile.TemporaryDirectory(prefix='orion-assets-') as temporary:
        source = prepare_catalog(app, Path(temporary) / 'catalog', version=version)
        apply_assets(root, source, System(), expected_release=app)
    print('Asset pack installed. The previous catalog remains available through --rollback.')

def main():
    parser = argparse.ArgumentParser(); parser.add_argument('--payload', type=Path); parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--codename', choices=('bookworm', 'trixie')); parser.add_argument('--rollback', action='store_true'); parser.add_argument('--check', action='store_true'); parser.add_argument('--assets', action='store_true'); parser.add_argument('--version')
    args = parser.parse_args()
    if args.assets:
        install_assets(args.root.resolve(), args.version)
    elif args.rollback:
        codename = read_platform()['version_codename']
        with deployment_lock(args.root):
            rollback(args.root, System(), before_restore=lambda folder, data: restore_native_packages(args.root, codename, folder, data))
    elif args.check:
        check_installation(args.root)
    elif args.payload and args.codename: install(args.payload.resolve(), args.root.resolve(), args.codename)
    else: parser.error('Select --payload and --codename, --rollback or --check')


if __name__ == '__main__':
    try:
        main()
    except (RuntimeError, OSError, subprocess.CalledProcessError) as error:
        if str(error) == UNAVAILABLE:
            raise SystemExit(UNAVAILABLE) from None
        raise SystemExit(f'Orion installation failed: {error}. Re-run the installer to resume an incomplete preparation.') from None
