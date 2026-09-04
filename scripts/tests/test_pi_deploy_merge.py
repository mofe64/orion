"""Exercise the streamed pre-merge upgrade against real, isolated Git histories."""
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'pi_deploy_remote.sh'
FUNCTION = re.search(r'^merge_updated_checkout\(\) \{\n.*?^\}', SCRIPT.read_text(), re.M | re.S).group()


class DeploymentMergeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.git('init', '-q')
        self.git('config', 'user.email', 'test@example.invalid')
        self.git('config', 'user.name', 'Deployment test')
        (self.root / 'README').write_text('initial')
        (self.root / 'voice').mkdir()
        self.git('add', 'README')
        self.git('commit', '-qm', 'Before tracked voice lockfile')
        self.base = self.git('rev-parse', 'HEAD').stdout.strip()

    def git(self, *args, check=True):
        return subprocess.run(['git', *args], cwd=self.root, text=True, capture_output=True, check=check)

    def incoming(self, lock=True):
        if lock:
            (self.root / 'voice/uv.lock').write_text('committed lockfile')
        (self.root / 'README').write_text('updated')
        self.git('add', '.')
        self.git('commit', '-qm', 'Incoming deployment')
        self.git('branch', 'incoming')
        self.git('reset', '--hard', self.base)
        (self.root / 'voice').mkdir(exist_ok=True)

    def merge(self):
        return subprocess.run(['bash', '-eu', '-c', FUNCTION + '\nmerge_updated_checkout incoming'],
                              cwd=self.root, text=True, capture_output=True)

    def test_untracked_generated_lock_is_replaced_and_rerun_is_idempotent(self):
        self.incoming()
        (self.root / 'voice/uv.lock').write_text('old generated dependencies')
        other = self.root / 'voice/operator-notes'; other.write_text('keep')
        result = self.merge()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.root / 'voice/uv.lock').read_text(), 'committed lockfile')
        self.assertEqual(other.read_text(), 'keep')
        self.assertEqual(self.merge().returncode, 0)

    def test_missing_lock_does_not_require_cleanup(self):
        self.incoming()
        self.assertEqual(self.merge().returncode, 0)

    def test_no_incoming_replacement_preserves_generated_lock(self):
        self.incoming(lock=False)
        lock = self.root / 'voice/uv.lock'; lock.write_text('keep')
        self.assertEqual(self.merge().returncode, 0)
        self.assertEqual(lock.read_text(), 'keep')

    def test_staged_lock_is_preserved_and_git_rejects_conflict(self):
        self.incoming()
        lock = self.root / 'voice/uv.lock'; lock.write_text('staged local work')
        self.git('add', 'voice/uv.lock')
        self.assertNotEqual(self.merge().returncode, 0)
        self.assertEqual(lock.read_text(), 'staged local work')

    def test_tracked_local_modification_is_preserved(self):
        lock = self.root / 'voice/uv.lock'; lock.write_text('old tracked lock')
        self.git('add', 'voice/uv.lock'); self.git('commit', '-qm', 'Tracked lock')
        self.base = self.git('rev-parse', 'HEAD').stdout.strip()
        self.incoming()
        lock.write_text('local edit')
        self.assertNotEqual(self.merge().returncode, 0)
        self.assertEqual(lock.read_text(), 'local edit')

    def test_staged_deletion_does_not_make_head_tracked_file_disposable(self):
        lock = self.root / 'voice/uv.lock'; lock.write_text('tracked')
        self.git('add', 'voice/uv.lock'); self.git('commit', '-qm', 'Tracked lock')
        self.base = self.git('rev-parse', 'HEAD').stdout.strip()
        self.incoming()
        self.git('rm', 'voice/uv.lock')
        lock.parent.mkdir(exist_ok=True)
        lock.write_text('untracked recreation')
        self.assertNotEqual(self.merge().returncode, 0)
        self.assertEqual(lock.read_text(), 'untracked recreation')

    def test_diverged_history_does_not_remove_lock(self):
        self.incoming()
        (self.root / 'README').write_text('local commit')
        self.git('add', 'README'); self.git('commit', '-qm', 'Local divergence')
        lock = self.root / 'voice/uv.lock'; lock.write_text('keep')
        self.assertNotEqual(self.merge().returncode, 0)
        self.assertEqual(lock.read_text(), 'keep')

    def test_symlink_is_not_removed(self):
        self.incoming()
        source = self.root / 'keep'; source.write_text('keep')
        lock = self.root / 'voice/uv.lock'; lock.symlink_to(source)
        self.assertNotEqual(self.merge().returncode, 0)
        self.assertTrue(lock.is_symlink())
        self.assertEqual(source.read_text(), 'keep')

    def test_unrelated_untracked_conflict_still_blocks_merge(self):
        self.incoming()
        self.git('checkout', 'incoming')
        (self.root / 'other').write_text('incoming')
        self.git('add', 'other'); self.git('commit', '-qm', 'Other incoming file')
        self.git('checkout', '--detach', self.base)
        (self.root / 'voice').mkdir(exist_ok=True)
        (self.root / 'voice/uv.lock').write_text('generated')
        (self.root / 'other').write_text('keep unrelated')
        self.assertNotEqual(self.merge().returncode, 0)
        self.assertEqual((self.root / 'other').read_text(), 'keep unrelated')


if __name__ == '__main__':
    unittest.main()
