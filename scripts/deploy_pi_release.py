#!/usr/bin/env python3
"""Install a locally built Pi bundle without compiling or retaining package caches."""
import argparse
import os
from pathlib import Path
import shutil
import shlex
import signal
import subprocess
import tarfile
import tempfile

from install_pi_voice_stack import deployment_lock
from pi_release_bundle import inspect_archive, install_wheels, sha256, verify_payload
from pi_release_platform import builder_image, check_platform, read_platform
from pi_service_config import listener_start, services_for_release
from prepare_pi_voice_stack import download, prepare_assets

YUNET_SHA256 = '8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4'
YUNET_URL = ('https://media.githubusercontent.com/media/opencv/opencv_zoo/'
             '47534e27c9851bb1128ccc0102f1145e27f23f98/models/face_detection_yunet/'
             'face_detection_yunet_2023mar.onnx')

PALM_SHA256 = '78ff51c38496b7fc8b8ebdb6cc8c1abb02fa6c38427c6848254cdaba57fcce7c'
PALM_URL = ('https://media.githubusercontent.com/media/opencv/opencv_zoo/'
            '47534e27c9851bb1128ccc0102f1145e27f23f98/models/palm_detection_mediapipe/'
            'palm_detection_mediapipe_2023feb.onnx')


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


def validate_listener(release):
    # Parse the same managed model arguments as systemd using the installed wheel.
    # A hard-coded smoke-test path can pass while the listener's defaults fail.
    arguments = shlex.split(listener_start('orion-listener --token-file /dev/null', release))[1:]
    run(release / 'voice/.venv/bin/python', '-c',
        'from pathlib import Path; from orion_voice.rustpotter import RustpotterWakeDetector; '
        'from orion_voice.satellite import parse_args; args = parse_args(); '
        'RustpotterWakeDetector(Path("voice/models/wake/hey_orion_trained_080.rpw"), .80).process(bytes(640)); '
        'RustpotterWakeDetector(args.wake_model, args.threshold).process(bytes(640)); '
        'import numpy as np; from orion_voice.verifier import AcousticVerifier; '
        'AcousticVerifier(args.verifier_dir).feed(np.zeros(1280, dtype=np.int16))', *arguments, cwd=release)


def prepare_vision(release):
    # Release-local data follows activation/rollback without replacing native libraries.
    download(release, YUNET_URL, 'vision/models/face_detection_yunet_2023mar.onnx', YUNET_SHA256)
    if (release / 'vision/orion_vision/detect_hands.py').is_file():
        download(release, PALM_URL, 'vision/models/palm_detection_mediapipe_2023feb.onnx', PALM_SHA256)
        run(release / 'vision/.venv/bin/python', '-c',
            'from orion_vision.detect_hands import PalmDetector; import sys; PalmDetector(sys.argv[1])',
            release / 'vision/models/palm_detection_mediapipe_2023feb.onnx')
    if (release / 'vision/orion_vision/detect_objects.py').is_file():
        from vision_assets import verify_yolo
        verify_yolo(release)
        run(release / 'vision/.venv/bin/python', '-c',
            'from orion_vision.detect_objects import YoloNCNN; import sys; YoloNCNN(sys.argv[1])',
            release / 'vision/models/yolo11n_ncnn_model')


def validate_voice(release, root):
    validate_listener(release)
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
                    if 'orion-vision' in services_for_release(release):
                        prepare_vision(release)
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
