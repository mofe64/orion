#!/usr/bin/env python3
"""Build a committed, target-compatible Pi release locally using Docker."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import uuid

from pi_release_bundle import install_wheels, pack_bundle, write_manifest
from pi_release_platform import builder_image, check_platform, read_platform
from pi_service_config import services_for_release
from deploy_pi_release import validate_listener


RUST_VERSION = '1.98.0'
UV_VERSION = '0.11.13'
BINARY_COMPONENTS = {'runtime': ('oriond', 'orion-trajectory'), 'orion-service': ('orion-service',)}
CACHE_LIMIT_BYTES = 6 * 1024**3


def run(*args, **kwargs):
    return subprocess.run([str(arg) for arg in args], check=True, **kwargs)


def snapshot_commit(source, revision, releases, hardware='v1'):
    commit = run('git', '-C', source, 'rev-parse', '--verify', f'{revision}^{{commit}}',
                 capture_output=True, text=True).stdout.strip()
    release_id = f'{commit[:12]}-{uuid.uuid4().hex[:8]}'
    release = releases / release_id
    release.mkdir(parents=True)
    try:
        with tempfile.TemporaryFile() as stream:
            run('git', '-C', source, 'archive', '--format=tar', commit, stdout=stream)
            stream.seek(0)
            with tarfile.open(fileobj=stream) as archive:
                archive.extractall(release, filter='data')
        (release / 'release.json').write_text(json.dumps({
            'revision': commit[:12], 'commit': commit, 'hardware': hardware, 'release_id': release_id,
        }, indent=2) + '\n')
        if hardware == 'v2' and (release / 'vision/orion_vision/detect_objects.py').is_file():
            from vision_assets import stage_yolo
            stage_yolo(source, release)
    except BaseException:
        shutil.rmtree(release)
        raise
    return release


def runtime_components(release):
    return ('voice', 'speech', 'vision') if 'orion-vision' in services_for_release(release) else ('voice', 'speech')


def package_python(release, component, wheels):
    """Resolve from the committed lock; compile any source distributions on the builder."""
    python = Path('/opt/wheels-311/bin/python' if component == 'speech' else '/opt/wheels-312/bin/python')
    wheels.mkdir(parents=True)
    requirements = wheels / 'locked-requirements.txt'
    run('uv', 'export', '--project', release / component, '--locked', '--no-emit-local',
        '--no-dev', '--no-annotate', '--no-header', '--output-file', requirements,
        stdout=subprocess.DEVNULL)
    run(python, '-m', 'pip', 'wheel', '--no-deps', '--require-hashes',
        '--wheel-dir', wheels, '--requirement', requirements)
    projects = [release / component]
    if component == 'voice':
        projects.insert(0, release / 'voice/rustpotter_native')
    for project in projects:
        run(python, '-m', 'pip', 'wheel', '--no-deps', '--wheel-dir', wheels, project)
    # Test precisely the wheels we will send, rather than editable source installs.
    environment = release / component / '.venv'
    run('uv', 'venv', '--python', python, environment)
    install_wheels(environment / 'bin/python', wheels, run)
    run(environment / 'bin/python', '-m', 'unittest', 'discover', '-s', release / component / 'tests', '-q', cwd=release)
    if component == 'voice':
        validate_listener(release)


def ignore_build_files(directory, names):
    return [name for name in names if name in {'.venv', '__pycache__', 'node_modules', 'target', 'build', 'dist'}
            or name.endswith('.egg-info')]


def trim_local_cache(cache=Path('/cache'), limit=CACHE_LIMIT_BYTES):
    """Bound this pipeline's local Docker cache; preserve the live lock inode."""
    seen = set()
    size = 0
    for path in cache.rglob('*'):
        if path.is_file() and not path.is_symlink():
            stat = path.stat()
            identity = (stat.st_dev, stat.st_ino)
            if identity not in seen:
                seen.add(identity)
                size += stat.st_size
    if size <= limit:
        return
    for path in cache.iterdir():
        if path.name == 'build.lock':
            continue
        if path.is_dir() and not path.is_symlink():
            shutil.rmtree(path)
        else:
            path.unlink()
    print('Cleared local builder caches exceeding 6 GiB; prepared artifacts are preserved.', flush=True)


def build_in_container(release, output, target):
    if os.uname().sysname != 'Linux' or os.uname().machine != 'aarch64':
        raise RuntimeError('The release builder must run in ARM64 Linux')
    builder_image(target)
    check_platform(target, read_platform())
    revision = json.loads((release / 'release.json').read_text())['revision']
    environment = {**os.environ, 'ORION_BUILD_REVISION': revision}
    run('python3', '-m', 'unittest', 'discover', '-s', release / 'scripts/tests', '-q', cwd=release, env=environment)
    for component in ('history', 'agent', 'coordinator', *BINARY_COMPONENTS):
        tests = ['cargo', 'test', '--locked', '--manifest-path', release / component / 'Cargo.toml', '--all-targets', '-j', '2']
        if component == 'runtime':
            tests += ['--', '--skip', 'native_mujoco']
        run(*tests, cwd=release, env=environment)
        if component in BINARY_COMPONENTS:
            run('cargo', 'build', '--release', '--locked', '--manifest-path', release / component / 'Cargo.toml',
                '-j', '2', cwd=release, env=environment)
    output.mkdir()
    for component in runtime_components(release):
        package_python(release, component, output / 'wheels' / component)
    run('python3', '-m', 'unittest', 'discover', '-s', release / 'orion_studio/tests', '-q', cwd=release)
    staged = output / 'release'
    shutil.copytree(release, staged, ignore=ignore_build_files)
    for component, binaries in BINARY_COMPONENTS.items():
        destination = staged / component / 'target/release'
        destination.mkdir(parents=True)
        for binary in binaries:
            shutil.copy2(Path(os.environ['CARGO_TARGET_DIR']) / 'release' / binary, destination / binary)
    write_manifest(output, staged, target)


def build_local(release, target, output, skip_studio=False):
    base = builder_image(target)
    if not skip_studio:
        run('pnpm', '--dir', release / 'orion_studio', 'install', '--frozen-lockfile')
        run('pnpm', '--dir', release / 'orion_studio', 'test')
        run('pnpm', '--dir', release / 'orion_studio', 'build')
    context = release / 'scripts'
    if not (context / 'PiRelease.Dockerfile').exists():
        raise RuntimeError('The selected commit has no local release builder; commit and push the pipeline changes first')
    image = f'orion-pi-builder:{target["id"]}-{target["version_id"]}-rust{RUST_VERSION}-uv{UV_VERSION}'
    run('docker', 'build', '--platform', 'linux/arm64', '--build-arg', f'BASE_IMAGE={base}',
        '--build-arg', f'RUST_VERSION={RUST_VERSION}', '--tag', image, '--file', context / 'PiRelease.Dockerfile', context)
    target_file = release.parent / 'target.json'
    target_file.write_text(json.dumps(target))
    container_source = Path('/work') / release.name
    bundle = release.parent / 'bundle'
    container = f'orion-pi-build-{uuid.uuid4().hex}'
    try:
        run('docker', 'run', '--rm', '--init', '--name', container, '--platform', 'linux/arm64',
            '--mount', f'type=bind,source={release.parent},target=/work',
            '--mount', 'type=volume,source=orion-pi-build-cache,target=/cache',
            image, 'flock', '/cache/build.lock', 'python3', container_source / 'scripts/build_pi_release.py',
            '--inside-container', container_source, '--output', '/work/bundle', '--target', '/work/target.json')
    finally:
        subprocess.run(['docker', 'rm', '-f', container], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    output.parent.mkdir(parents=True, exist_ok=True)
    partial = output.with_name(output.name + '.partial')
    try:
        pack_bundle(bundle, partial)
        partial.replace(output)
    finally:
        partial.unlink(missing_ok=True)
    print(f'Prepared {output} for {target["id"]} {target["version_id"]} aarch64', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, default=Path.cwd())
    parser.add_argument('--revision')
    parser.add_argument('--hardware', choices=('v1', 'v2'))
    parser.add_argument('--target', type=Path, help='Robot platform JSON from pi_release_platform.py --probe')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--skip-studio-check', action='store_true')
    parser.add_argument('--inside-container', type=Path)
    parser.add_argument('--prepared-source', type=Path, help=argparse.SUPPRESS)
    parser.add_argument('--snapshot-only', action='store_true', help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.snapshot_only:
        if not args.revision or not args.hardware:
            parser.error('--revision and --hardware are required')
        print(snapshot_commit(args.source.resolve(), args.revision, args.output.resolve(), args.hardware))
        return
    if not args.target:
        parser.error('--target is required')
    target = json.loads(args.target.read_text())
    if args.inside_container:
        trim_local_cache()
        try:
            build_in_container(args.inside_container, args.output, target)
        finally:
            trim_local_cache()
        return
    if args.output.exists():
        raise RuntimeError(f'Refusing to overwrite {args.output}')
    if args.prepared_source:
        build_local(args.prepared_source, target, args.output.resolve(), args.skip_studio_check)
        return
    if not args.revision or not args.hardware:
        parser.error('--revision and --hardware are required')
    with tempfile.TemporaryDirectory(prefix='orion-local-release-') as directory:
        release = snapshot_commit(args.source.resolve(), args.revision, Path(directory), args.hardware)
        build_local(release, target, args.output.resolve(), args.skip_studio_check)


if __name__ == '__main__':
    main()
