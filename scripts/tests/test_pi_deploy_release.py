"""Committed releases must not merge over operator edits in the Pi checkout."""
from pathlib import Path
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
from build_pi_release import snapshot_commit
import build_pi_release
from pi_release_platform import TARGET_PLATFORM
from types import SimpleNamespace


class DeploymentSnapshotTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.root = self.base / 'checkout'; self.root.mkdir()
        self.git('init', '-q')
        self.git('config', 'user.email', 'test@example.invalid')
        self.git('config', 'user.name', 'Deployment test')
        (self.root / 'scripts').mkdir()
        (self.root / 'scripts/deploy_pi_release.py').write_text('committed deployment')
        (self.root / 'README').write_text('initial')
        self.git('add', '.'); self.git('commit', '-qm', 'Initial')
        self.initial = self.git('rev-parse', 'HEAD').stdout.strip()
        (self.root / 'README').write_text('incoming')
        (self.root / 'voice').mkdir()
        (self.root / 'voice/uv.lock').write_text('committed lock')
        self.git('add', '.'); self.git('commit', '-qm', 'Incoming')
        self.incoming = self.git('rev-parse', 'HEAD').stdout.strip()
        self.git('checkout', '-q', '--detach', self.initial)
        (self.root / 'README').write_text('operator edit')
        (self.root / 'scripts/deploy_pi_release.py').write_text('uncommitted deployment')
        (self.root / 'voice').mkdir(exist_ok=True)
        (self.root / 'voice/uv.lock').write_text('generated lock')
        (self.root / 'keep').write_text('untracked')

    def git(self, *args):
        return subprocess.run(['git', *args], cwd=self.root, check=True, capture_output=True, text=True)

    def test_snapshot_selects_commit_without_changing_dirty_files_index_or_head(self):
        self.git('add', 'README')
        before = {p.relative_to(self.root): p.read_bytes() for p in self.root.rglob('*') if p.is_file() and '.git' not in p.parts}
        status = self.git('status', '--porcelain').stdout
        release = snapshot_commit(self.root, self.incoming, self.base / 'releases')
        self.assertEqual((release / 'README').read_text(), 'incoming')
        self.assertEqual((release / 'voice/uv.lock').read_text(), 'committed lock')
        self.assertFalse((release / 'keep').exists())
        self.assertEqual(self.git('rev-parse', 'HEAD').stdout.strip(), self.initial)
        self.assertEqual(self.git('status', '--porcelain').stdout, status)
        for path, content in before.items():
            self.assertEqual((self.root / path).read_bytes(), content)

    def test_bundle_source_uses_committed_deployment_helpers(self):
        release = snapshot_commit(self.root, self.incoming, self.base / 'releases')
        self.assertEqual((release / 'scripts/deploy_pi_release.py').read_text(), 'committed deployment')

    def test_invalid_revision_creates_no_release(self):
        with self.assertRaises(subprocess.CalledProcessError):
            snapshot_commit(self.root, 'missing-ref', self.base / 'releases')
        self.assertFalse((self.base / 'releases').exists())

    def test_each_build_gets_a_separate_permanent_directory(self):
        first = snapshot_commit(self.root, self.incoming, self.base / 'releases')
        second = snapshot_commit(self.root, self.incoming, self.base / 'releases')
        self.assertNotEqual(first, second)
        self.assertTrue(first.is_dir())


class ReleaseBuildTests(unittest.TestCase):
    def test_small_local_cache_is_kept_and_hardlinks_are_counted_once(self):
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory)
            (cache / 'wheel').write_bytes(bytes(20))
            os.link(cache / 'wheel', cache / 'linked-wheel')
            build_pi_release.trim_local_cache(cache, limit=25)
            self.assertTrue((cache / 'wheel').exists())
            self.assertTrue((cache / 'linked-wheel').exists())

    def test_large_local_cache_is_cleared_without_replacing_lock_or_following_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cache = root / 'cache'; cache.mkdir()
            lock = cache / 'build.lock'; lock.touch()
            identity = lock.stat().st_ino
            (cache / 'target').mkdir()
            (cache / 'target/binary').write_bytes(bytes(20))
            outside = root / 'outside'; outside.mkdir()
            (outside / 'keep').write_text('keep')
            (cache / 'linked-directory').symlink_to(outside)
            build_pi_release.trim_local_cache(cache, limit=10)
            self.assertEqual(list(cache.iterdir()), [lock])
            self.assertEqual(lock.stat().st_ino, identity)
            self.assertEqual((outside / 'keep').read_text(), 'keep')

    def test_local_build_keeps_runtime_tests_and_packages_only_runtime_binaries(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            release = root / 'release'; release.mkdir()
            shutil.copytree(SCRIPTS.parent / 'hardware/profiles', release / 'hardware/profiles')
            (release / 'release.json').write_text(json.dumps({
                'revision': 'a' * 12, 'commit': 'a' * 40, 'hardware': 'v2', 'release_id': 'a' * 12 + '-12345678'}))
            cache = root / 'cache/release'; cache.mkdir(parents=True)
            for component, binaries in build_pi_release.BINARY_COMPONENTS.items():
                for binary in binaries:
                    (cache / binary).write_text('production binary')
            for name in ['runtime/target/debug/debris', 'voice/.venv/debris', 'voice/build/debris',
                         'orion_studio/node_modules/debris', 'scripts/__pycache__/debris']:
                path = release / name; path.parent.mkdir(parents=True, exist_ok=True); path.write_text('cache')
            with patch.object(build_pi_release, 'run') as run, \
                 patch.object(build_pi_release, 'package_python') as packages, \
                 patch.object(build_pi_release.os, 'uname', return_value=SimpleNamespace(sysname='Linux', machine='aarch64')), \
                 patch.object(build_pi_release, 'read_platform', return_value=TARGET_PLATFORM), \
                 patch.dict(build_pi_release.os.environ, CARGO_TARGET_DIR=str(cache.parent)):
                build_pi_release.build_in_container(release, root / 'bundle', TARGET_PLATFORM)
            self.assertEqual([call.args[1] for call in packages.call_args_list], ['voice', 'speech', 'vision'])
            staged = root / 'bundle/release'
            for name in ['runtime/target/debug', 'voice/.venv', 'voice/build', 'orion_studio/node_modules', 'scripts/__pycache__']:
                self.assertFalse((staged / name).exists(), name)
            self.assertEqual((staged / 'runtime/target/release/oriond').read_text(), 'production binary')
        commands = [tuple(map(str, call.args)) for call in run.call_args_list]
        runtime = next(args for args in commands if args[:2] == ('cargo', 'test')
                       and str(release / 'runtime/Cargo.toml') in args)
        skips = [runtime[i + 1] for i, arg in enumerate(runtime) if arg == '--skip']
        for name in (
            'devices::mujoco::tests::rust_runtime_executes_and_settles_in_native_mujoco',
            'expression::character::tests::rust_runtime_executes_and_settles_in_native_mujoco_character_animation',
        ):
            self.assertTrue(any(skip in name for skip in skips), name)
        for name in ('expression::character::tests::streamed_speech_extends_without_resetting_anchor_or_settling_between_chunks',
                     'expression::speech::tests::underrun_and_upload_timeout_have_distinct_diagnostics',
                     'control::core::tests::enforces_configuration_and_torque_lifecycle'):
            self.assertFalse(any(skip in name for skip in skips), name)
        self.assertIn('--all-targets', runtime)
        builds = [command for command in commands if command[:2] == ('cargo', 'build')]
        self.assertEqual(len(builds), 2)
        self.assertTrue(all('--release' in command and '--locked' in command for command in builds))
        service = next(args for args in commands if args[:2] == ('cargo', 'test')
                       and str(release / 'orion-service/Cargo.toml') in args)
        self.assertNotIn('--skip', service)


if __name__ == '__main__':
    unittest.main()
