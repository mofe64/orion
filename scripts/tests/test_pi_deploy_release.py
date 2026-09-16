"""Committed releases must not merge over operator edits in the Pi checkout."""
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
from deploy_pi_release import snapshot_commit

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


if __name__ == '__main__':
    unittest.main()
