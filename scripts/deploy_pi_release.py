#!/usr/bin/env python3
"""Install a locally built Pi bundle without compiling or retaining package caches."""
import argparse
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tarfile
import tempfile

from install_pi_voice_stack import deployment_lock
from pi_release_bundle import inspect_archive, install_wheels, sha256, verify_payload
from pi_release_platform import builder_image, check_platform, read_platform
from pi_service_config import services_for_release
from prepare_pi_voice_stack import prepare_assets


def run(*args, **kwargs):
    return subprocess.run([str(arg) for arg in args], check=True, **kwargs)


def prepare_environments(release, wheels):
    components = ('voice', 'speech', 'vision') if 'orion-vision' in services_for_release(release) else ('voice', 'speech')
    for component in components:
        python = '3.11' if component == 'speech' else '/usr/bin/python3'
        extra = ['--system-site-packages'] if component == 'vision' else []
        environment = release / component / '.venv'
        run('uv', 'venv', '--no-config', '--offline', '--no-cache', '--no-python-downloads',
            '--python', python, *extra, environment)
        install_wheels(environment / 'bin/python', wheels / component, run)
    if 'vision' in components:
        run(release / 'vision/.venv/bin/python', '-c', 'import libcamera, pykms, picamera2')


def validate_voice(release, root):
    run(release / 'voice/.venv/bin/python', '-c',
        'from pathlib import Path; from orion_voice.rustpotter import RustpotterWakeDetector; '
        'RustpotterWakeDetector(Path("voice/models/wake/hey_orion_trained_080.rpw"), .80).process(bytes(640)); '
        'RustpotterWakeDetector(Path("voice/models/wake/hey_orion_reference.rpw"), .35).process(bytes(640)); '
        'import numpy as np; from orion_voice.verifier import AcousticVerifier; '
        'AcousticVerifier(Path("voice/models/verifier")).feed(np.zeros(1280, dtype=np.int16))', cwd=release)
    environment = {**os.environ, 'HF_HOME': str(root / 'cache/hf'), 'HF_HUB_OFFLINE': '1',
                   'ORION_SPEECH_BACKEND': 'pi', 'PYTHONPATH': str(release / 'speech'),
                   'ORION_TTS_THREADS': '3', 'ORION_PIPER_MODEL_DIR': str(root / 'models/piper-alba-medium')}
    run(release / 'speech/.venv/bin/python', '-c',
        "from orion_speech_worker.piper import PiperAlbaSynthesizer; "
        "audio=list(PiperAlbaSynthesizer('piper-alba-medium').stream('Orion is ready.')); "
        "assert audio and all(chunk.sample_rate == 24000 for chunk in audio)", env=environment, cwd=release)


def prepare_bundle(bundle, digest, root, hardware):
    if sha256(bundle) != digest:
        raise RuntimeError('Transferred release checksum does not match; no services were changed')
    with tarfile.open(bundle) as archive:
        manifest, required = inspect_archive(archive)
        if manifest['hardware'] != hardware:
            raise RuntimeError('Bundle hardware selection does not match the deployment')
        builder_image(manifest['platform'])
        check_platform(manifest['platform'], read_platform())
        with deployment_lock(root):
            if (root / 'pending-installation.json').exists():
                raise RuntimeError('Recover the interrupted deployment with install_pi_voice_stack.py --rollback first')
            free = shutil.disk_usage(root).free
            if free < required:
                raise RuntimeError(f'Only {free / 1e9:.1f} GB free; unpacking and installing this bundle needs '
                                   f'{required / 1e9:.1f} GB, including reserved headroom')
            release = root / 'releases' / manifest['release_id']
            if release.exists():
                raise RuntimeError(f'Release already exists: {release}')
            with tempfile.TemporaryDirectory(dir=root, prefix='.incoming-') as directory:
                staging = Path(directory)
                archive.extractall(staging, filter='data')
                verify_payload(staging, manifest)
                release.parent.mkdir(exist_ok=True)
                (staging / 'release').rename(release)
                try:
                    prepare_environments(release, staging / 'wheels')
                    # Models/native inference tools are shared runtime assets; this does not build code.
                    prepare_assets(root)
                    validate_voice(release, root)
                except BaseException:
                    shutil.rmtree(release)
                    raise
            print(f'Prepared {release}. No compilation ran on the Pi.', flush=True)
            return release


def interrupted(signum, frame):
    raise KeyboardInterrupt('Deployment interrupted')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--sha256', required=True)
    parser.add_argument('--hardware', choices=('v1', 'v2'), required=True)
    parser.add_argument('--root', type=Path, default=Path.home() / '.local/share/orion/voice-stack')
    parser.add_argument('--runtime-project', type=Path, default=Path.home() / 'orion')
    parser.add_argument('--prepare-only', action='store_true')
    args = parser.parse_args()
    if os.uname().sysname != 'Linux' or os.uname().machine != 'aarch64' or os.geteuid() == 0:
        raise RuntimeError('Run this command as the Pi user on ARM64 Linux')
    if not shutil.which('uv'):
        raise RuntimeError('Install uv and Python 3.11 before installing a Pi release')
    signal.signal(signal.SIGTERM, interrupted)
    release = prepare_bundle(args.bundle.resolve(), args.sha256, args.root.resolve(), args.hardware)
    if args.prepare_only:
        print(f'Activate with python3 {release}/scripts/install_pi_voice_stack.py --release {release} '
              f'--runtime-project {args.runtime_project.resolve()}', flush=True)
        return
    run('/usr/bin/python3', release / 'scripts/install_pi_voice_stack.py', '--root', args.root.resolve(),
        '--release', release, '--runtime-project', args.runtime_project.resolve())


if __name__ == '__main__':
    main()
