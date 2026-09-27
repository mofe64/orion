"""Committed releases must not merge over operator edits in the Pi checkout."""
from pathlib import Path
import json
import re
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
from deploy_pi_release import snapshot_commit
import deploy_pi_release

FUNCTION = re.search(r'^extract_revision_scripts\(\) \{\n.*?^\}',
                     (SCRIPTS / 'pi_deploy_remote.sh').read_text(), re.M | re.S).group()


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

    def test_remote_bootstrap_uses_committed_script(self):
        destination = self.base / 'bootstrap'; destination.mkdir()
        result = subprocess.run(['bash', '-euo', 'pipefail', '-c', FUNCTION + '\nextract_revision_scripts "$1" "$2"',
                                 'test', self.incoming, str(destination)], cwd=self.root, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((destination / 'scripts/deploy_pi_release.py').read_text(), 'committed deployment')

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
    def test_pi_skips_both_native_simulations_and_keeps_other_runtime_tests(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            release = root / 'release'; release.mkdir()
            (release / 'release.json').write_text(json.dumps({'revision': 'fixture'}))
            with patch.object(deploy_pi_release, 'run') as run, \
                 patch.object(deploy_pi_release.shutil, 'copy2'):
                deploy_pi_release.build_release(release, root)
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
        service = next(args for args in commands if args[:2] == ('cargo', 'test')
                       and str(release / 'orion-service/Cargo.toml') in args)
        self.assertNotIn('--skip', service)


if __name__ == '__main__':
    unittest.main()
