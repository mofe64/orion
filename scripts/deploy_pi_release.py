#!/usr/bin/env python3
"""Build a complete immutable Pi release from a Git commit, then activate it."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import uuid

from install_pi_voice_stack import deployment_lock


def run(*args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def snapshot_commit(source, revision, releases):
    commit = run('git', '-C', source, 'rev-parse', '--verify', f'{revision}^{{commit}}', capture_output=True, text=True).stdout.strip()
    release = releases / f'{commit[:12]}-{uuid.uuid4().hex[:8]}'
    release.mkdir(parents=True)
    try:
        with tempfile.TemporaryFile() as archive:
            run('git', '-C', source, 'archive', '--format=tar', commit, stdout=archive)
            archive.seek(0)
            with tarfile.open(fileobj=archive) as package:
                package.extractall(release, filter='data')
        (release / 'release.json').write_text(json.dumps({'revision': commit[:12], 'commit': commit, 'source': str(source)}, indent=2))
    except BaseException:
        shutil.rmtree(release)
        raise
    return release


def build_release(release, root):
    env = dict(os.environ, ORION_BUILD_REVISION=json.loads((release / 'release.json').read_text())['revision'])
    # Use a separate shared build cache, copying binaries into their permanent release paths.
    # Neither the old runtime binary nor the old Python environments are overwritten.
    env['CARGO_TARGET_DIR'] = str(root / 'build')
    run('python3', '-m', 'unittest', 'discover', '-s', release / 'scripts/tests', '-q', cwd=release)
    run('python3', release / 'scripts/prepare_pi_voice_stack.py', '--root', root, env=env)
    for component in ('runtime', 'orion-service'):
        # Build beside the live voice services on an 8 GB Pi; limit peak memory.
        args = ['cargo', 'test', '--locked', '--manifest-path', str(release / component / 'Cargo.toml'), '--all-targets', '-j', '1']
        if component == 'runtime':
            args += ['--', '--skip', 'devices::mujoco::tests::rust_runtime_executes_and_settles_in_native_mujoco']
        run(*args, cwd=release, env=env)
        run('cargo', 'build', '--release', '--locked', '--manifest-path', release / component / 'Cargo.toml', '-j', '1', cwd=release, env=env)
    for component, binaries in [('runtime', ['oriond', 'orion-trajectory']), ('orion-service', ['orion-service'])]:
        target = release / component / 'target/release'; target.mkdir(parents=True)
        for binary in binaries:
            shutil.copy2(root / 'build/release' / binary, target / binary)
    for component in ('voice', 'speech'):
        run(release / component / '.venv/bin/python', '-m', 'unittest', 'discover', '-s', release / component / 'tests', '-q', cwd=release)
    run('python3', '-m', 'unittest', 'discover', '-s', release / 'orion_studio/tests', '-q', cwd=release)
    run(release / 'voice/.venv/bin/python', '-c',
        'from pathlib import Path; from orion_voice.rustpotter import RustpotterWakeDetector; '
        'RustpotterWakeDetector(Path("voice/models/wake/hey_orion_reference.rpw"), .35).process(bytes(640))', cwd=release)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--revision', required=True, help='Committed Git ref; working-tree edits are never discarded or deployed implicitly')
    parser.add_argument('--root', type=Path, default=Path.home() / '.local/share/orion/voice-stack')
    parser.add_argument('--runtime-project', type=Path, default=Path.home() / 'dev/orion', help='Existing motion/user-asset catalog; kept unchanged')
    parser.add_argument('--prepare-only', action='store_true')
    args = parser.parse_args()
    if os.uname().machine != 'aarch64' or os.geteuid() == 0:
        raise RuntimeError('Run this command as the Pi user on 64-bit Linux')
    source, root = args.source.resolve(), args.root.resolve()
    for tool in ('cargo', 'git', 'python3', 'uv'):
        if not shutil.which(tool):
            raise RuntimeError(f'Install {tool} before preparing a Pi release')
    with deployment_lock(root):
        if (root / 'pending-installation.json').exists():
            raise RuntimeError('Recover the interrupted deployment with install_pi_voice_stack.py --rollback first')
        release = snapshot_commit(source, args.revision, root / 'releases')
        print(f'Preparing {release}. Existing services and the source checkout remain unchanged.', flush=True)
        try:
            build_release(release, root)
        except BaseException:
            # Only this newly-created, never-activated release is disposable.
            shutil.rmtree(release)
            raise
    if args.prepare_only:
        print(f'Prepared {release}; activate with scripts/install_pi_voice_stack.py --release {release}')
        return
    # The installer takes its own lock and captures the current state immediately before switching.
    run('python3', release / 'scripts/install_pi_voice_stack.py', '--root', root,
        '--release', release, '--runtime-project', args.runtime_project.resolve())


if __name__ == '__main__':
    main()
