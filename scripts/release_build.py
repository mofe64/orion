"""Build ARM64 Bookworm payload plus OS-specific Python 3.12 native wheels."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import uuid

from prepare_pi_voice_stack import LLAMA_VERSION

ROOT = Path('/src')
OUT = Path('/out')
COMPONENTS = ('speech', 'voice', 'vision', 'motion', 'orion_studio')
BINARY_COMPONENTS = {'runtime': ('oriond', 'orion-trajectory'), 'orion-service': ('orion-service',)}


def run(*args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def wheel(*args, destination):
    destination.mkdir(parents=True, exist_ok=True)
    run(sys.executable, '-m', 'pip', 'wheel', '--no-deps', '--wheel-dir', destination, *args)


def binding_wrapper(name, revision):
    source = Path('/tmp') / name
    run('git', 'clone', f'https://github.com/raspberrypi/{name}.git', source)
    run('git', '-C', source, 'checkout', revision)
    clone = source / ('clone-libcamera.sh' if name == 'pylibcamera' else 'clone-kmsxx.sh')
    clone.write_text(clone.read_text().replace('#!/bin/bash', '#!/bin/bash\nset -euo pipefail'))
    return source


def native(codename):
    manifest = json.loads((ROOT / 'scripts/release-native.json').read_text())
    pins = manifest[codename]
    # Never install the driver in a container. The binding links against apt's
    # userspace runtime, with exactly the same version installed on the Pi.
    run('apt-get', 'update')
    run('apt-get', 'install', '-y', '--no-install-recommends', 'hailort=' + ('4.20.0-1' if codename == 'bookworm' else '4.23.0'))
    out = OUT / 'payload/wheels/native' / codename
    camera = binding_wrapper('pylibcamera', manifest['libcamera_wrapper_revision'])
    # Both pinned libcamera revisions moved Python dependency discovery into
    # src/py. Stop its extension definition so meson-python owns wheel placement.
    shutil.copy2(ROOT / 'scripts/native/libcamera-meson.patch', camera / 'pypatch.patch')
    wheel(camera, '-C', 'setup-args=-Drevision=' + pins['libcamera_revision'],
          '-C', 'setup-args=-Dversion=unknown', '-C', 'setup-args=-Dcpp_std=c++20',
          '-C', 'setup-args=-Dwerror=false',
          '-C', 'compile-args=-j2', destination=out)
    kms = binding_wrapper('pykms', manifest['kms_wrapper_revision'])
    shutil.copy2(ROOT / 'scripts/native/kms-meson.patch', kms / 'pypatch.patch')
    wheel(kms, '-C', 'setup-args=-Drevision=' + manifest['kms_source_revision'],
          '-C', 'setup-args=-Dcpp_std=c++20', '-C', 'compile-args=-j2', destination=out)
    source = Path('/tmp/hailort')
    run('git', 'clone', '--depth', '1', '--branch', 'v' + pins['hailort_version'],
        'https://github.com/hailo-ai/hailort.git', source)
    wheel(source / 'hailort/libhailort/bindings/python/platform', destination=out)
    # Build/download the Python dependencies of this OS's exact Hailo wheel,
    # constrained to the common vision lock's NumPy. No native bindings leak
    # into the common wheel directory.
    run(sys.executable, '-m', 'pip', 'wheel', '--wheel-dir', out,
        next(out.glob('hailort-*.whl')), 'numpy==1.26.4')
    environment = Path('/tmp/native-' + codename)
    run('uv', 'venv', '--python', sys.executable, environment)
    from pi_release_bundle import install_wheels
    install_wheels(environment / 'bin/python', OUT / 'payload/wheels/vision', run)
    run('uv', 'pip', 'install', '--python', environment / 'bin/python', '--offline', '--no-index', '--no-build', '--no-deps', *sorted(out.glob('*.whl')))
    run(environment / 'bin/python', '-c', 'import libcamera, pykms, picamera2, hailo_platform', cwd=environment)
    version = run('dpkg-query', '-W', '-f=${Version}', 'libcamera-dev', capture_output=True, text=True).stdout
    if version != pins['libcamera_version']:
        raise RuntimeError(f'{codename} camera ABI changed: {version}; update release-native.json and rebuild')
    kms_version = run('dpkg-query', '-W', '-f=${Version}', 'libkms++-dev', capture_output=True, text=True).stdout
    if kms_version != pins['kms_version']: raise RuntimeError(f'{codename} KMS ABI changed: {kms_version}')
    (out / 'abi.json').write_text(json.dumps(pins, indent=2) + '\n')


def shared():
    payload = OUT / 'payload'
    if payload.exists(): shutil.rmtree(payload)
    release = payload / 'release'
    shutil.copytree(ROOT, release, ignore=shutil.ignore_patterns('.venv', '.git', 'target', 'node_modules',
        '__pycache__', 'artifacts', '*.egg-info', '.pytest_cache', '.scratch', 'models'))
    run('uv', 'venv', '--python', sys.executable, ROOT / '.venv')
    run('uv', 'pip', 'install', '--python', ROOT / '.venv/bin/python', '-r', ROOT / 'simulation/mujoco/requirements.txt', 'pytest==9.1.1', 'pillow')
    commit = os.environ['ORION_BUILD_REVISION']
    metadata = {'release_id': commit[:12] + '-' + uuid.uuid4().hex[:8], 'commit': commit,
                'source_dirty': os.environ.get('ORION_SOURCE_DIRTY') == 'true', 'revision': commit[:12], 'hardware': 'v2', 'joint_frame': 'mechanical_rest', 'version': os.environ['ORION_RELEASE_VERSION']}
    (release / 'release.json').write_text(json.dumps(metadata, indent=2) + '\n')
    for component in ('history', 'agent', 'coordinator', *BINARY_COMPONENTS):
        (ROOT / component / 'target').symlink_to(Path(os.environ['CARGO_TARGET_DIR']), target_is_directory=True)
        run('cargo', 'test', '--locked', '--manifest-path', ROOT / component / 'Cargo.toml', '--', '--test-threads=2')
        if component in BINARY_COMPONENTS:
            run('cargo', 'build', '--release', '--locked', '--manifest-path', ROOT / component / 'Cargo.toml')
            dest = release / component / 'target/release'; dest.mkdir(parents=True)
            for binary in BINARY_COMPONENTS[component]:
                shutil.copy2(Path(os.environ['CARGO_TARGET_DIR']) / 'release' / binary, dest / binary)
    if shutil.disk_usage(OUT).free < 4 * 1024**3:
        shutil.rmtree(Path(os.environ['CARGO_TARGET_DIR']) / 'debug', ignore_errors=True)
    for component in COMPONENTS:
        out = payload / 'wheels' / component; out.mkdir(parents=True, exist_ok=True)
        requirements = out / 'locked-requirements.txt'
        run('uv', 'export', '--project', ROOT / component, '--locked', '--no-emit-local', '--no-dev',
            '--no-annotate', '--no-header', '--output-file', requirements)
        wheel('--require-hashes', '-r', requirements, destination=out)
        if component == 'voice': wheel(ROOT / 'voice/rustpotter_native', destination=out)
        wheel(ROOT / component, destination=out)
        env = Path('/tmp/test-' + component)
        run('uv', 'venv', '--python', sys.executable, env)
        sys.path.insert(0, str(ROOT / 'scripts'))
        from pi_release_bundle import install_wheels
        install_wheels(env / 'bin/python', out, run)
        test_dir = 'test' if component == 'motion' else 'tests'
        if component == 'motion':
            run('uv', 'pip', 'install', '--python', env / 'bin/python', 'pytest==9.1.1')
            run(env / 'bin/python', '-m', 'pytest', ROOT / component / test_dir, '-q', cwd=ROOT,
                env={**os.environ, 'ORION_TRAJECTORY_COMPILER': str(release / 'runtime/target/release/orion-trajectory')})
        else:
            run(env / 'bin/python', '-m', 'unittest', 'discover', '-s', ROOT / component / test_dir, cwd=ROOT)
    run(sys.executable, '-m', 'unittest', 'discover', '-s', ROOT / 'scripts/tests', cwd=ROOT)
    run(ROOT / '.venv/bin/python', '-m', 'pytest', ROOT / 'tests', '-q', cwd=ROOT)
    # Build the GGUF server against glibc 2.36, rather than downloading an Ubuntu binary.
    source = Path('/tmp/llama.cpp')
    run('git', 'clone', '--depth', '1', '--branch', LLAMA_VERSION, 'https://github.com/ggml-org/llama.cpp.git', source)
    run('cmake', '-S', source, '-B', source / 'build', '-DCMAKE_BUILD_TYPE=Release',
        '-DBUILD_SHARED_LIBS=OFF', '-DGGML_NATIVE=OFF', '-DLLAMA_CURL=OFF', '-DLLAMA_BUILD_TESTS=OFF')
    run('cmake', '--build', source / 'build', '--target', 'llama-server', '-j', '2')
    dest = release / 'native'; dest.mkdir()
    run('git', 'clone', '--branch', 'pi5', 'https://github.com/jgarff/rpi_ws281x.git', dest / 'rpi_ws281x')
    run('git', '-C', dest / 'rpi_ws281x', 'checkout', '1396df4f35a86de0f7e5eda91d94d4539eef1727')
    shutil.rmtree(dest / 'rpi_ws281x/.git')
    shutil.copy2(source / 'build/bin/llama-server', dest / 'llama-server')
    # The image already requires managed Python. Resolve through any active
    # venv to copy the distribution, rather than the builder/test environment.
    python_root = Path(run('uv', 'python', 'find', '--no-project', '--resolve-links', '3.12.13',
                           capture_output=True, text=True).stdout.strip()).parent.parent
    if (payload / 'python').exists(): shutil.rmtree(payload / 'python')
    shutil.copytree(python_root, payload / 'python', symlinks=False)
    (payload / 'bin').mkdir(exist_ok=True)
    shutil.copy2(shutil.which('uv'), payload / 'bin/uv')


def pack():
    sys.path.insert(0, str(ROOT / 'scripts'))
    from pi_release_bundle import write_manifest, pack_bundle, sha256
    payload = OUT / 'payload'
    write_manifest(payload, payload / 'release', {'machine': 'aarch64', 'libc_minimum': '2.36',
        'version_codename': 'bookworm', 'id': 'raspbian'})
    sums = ''.join(f'{sha256(p)}  {p.relative_to(payload).as_posix()}\n'
                   for p in sorted(payload.rglob('*')) if p.is_file() and p.name != 'SHA256SUMS')
    (payload / 'SHA256SUMS').write_text(sums)
    archive = Path(os.environ.get('ORION_ARTIFACT_DIR', str(OUT))) / f'orion-{os.environ["ORION_RELEASE_VERSION"].removeprefix("v")}-arm64.tar.gz'
    temporary = archive.with_name(archive.name + '.partial')
    pack_bundle(payload, temporary)
    temporary.replace(archive)
    archive.with_name(archive.name + '.sha256').write_text(sha256(archive) + '  ' + archive.name + '\n')
    print(archive)


def main():
    parser = argparse.ArgumentParser(); parser.add_argument('--codename', choices=('bookworm', 'trixie')); parser.add_argument('--pack', action='store_true')
    args = parser.parse_args()
    if args.pack: pack(); return
    if args.codename == 'bookworm': shared()
    native(args.codename)


if __name__ == '__main__': main()
