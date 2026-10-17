"""Asset release isolation, integrity, compatibility and interrupted updates."""
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
import release_assets as assets
import release_install as receiver
from release_plan import classify, version
from pi_release_bundle import sha256
from test_pi_release_install import FakeSystem, Fixture


class SelectionTests(unittest.TestCase):
    def test_asset_only_boundary_includes_version_and_excludes_users_and_code(self):
        for path in ('motion/config/v2/poses.yaml', 'motion/motions/v2/nod.yaml', 'scenes/v2/a.yaml', 'audio/cues/a.wav'):
            self.assertEqual(classify([path, 'ASSETS_VERSION']), 'assets')
            for other in ('VERSION', 'scripts/install.sh', 'motion/motions/v2/user/a.yaml', 'scenes/v1/user/a.yaml'):
                self.assertEqual(classify([path, other]), 'app')
        self.assertEqual(classify(['ASSETS_VERSION']), 'app')

    def test_strict_version_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ('VERSION', 'ASSETS_VERSION'): (root / name).write_text('1.2.3\n')
            self.assertEqual(version(root, 'app'), 'v1.2.3')
            self.assertEqual(version(root, 'assets'), 'assets-v1.2.3')
            for bad in ('01.2.3', 'v1.2.3', '1.2.3-test.1', '1.2'):
                (root / 'VERSION').write_text(bad)
                with self.assertRaises(ValueError): version(root, 'app')


class PackTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(); self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name); self.source = self.root / 'source'; self.source.mkdir()
        for relative, value in {'ASSETS_VERSION': '1.2.3', assets.PROFILE: '{}',
            'motion/config/v1/poses.yaml': 'poses', 'motion/config/v2/poses.yaml': 'poses',
            'motion/motions/v2/nod.yaml': 'motion', 'scenes/v2/a.yaml': 'scene',
            'audio/cues/a.wav': 'binary', 'motion/motions/v2/user/custom.yaml': 'private',
            'scenes/v2/user/custom.yaml': 'private', 'audio/cues/user/private.wav': 'private',
            'scripts/code.py': 'code'}.items():
            path = self.source / relative; path.parent.mkdir(parents=True, exist_ok=True); path.write_text(value)
        with patch.object(assets, 'compile_catalog'):
            self.archive = assets.build_pack(self.source, self.source, self.root / 'out')
        self.unpacked = self.root / 'unpacked'

    def test_round_trip_inventory_audio_and_profile(self):
        assets.unpack_verified(self.archive, self.unpacked, self.archive.with_name(self.archive.name + '.sha256').read_bytes())
        manifest = assets.verify_pack(self.unpacked, self.source)
        self.assertEqual(manifest['joint_profile_sha256'], sha256(self.source / assets.PROFILE))
        self.assertIn('audio/cues/a.wav', manifest['files'])
        self.assertFalse(any('user' in Path(p).parts for p in manifest['files']))
        self.assertNotIn('scripts/code.py', manifest['files'])
        (self.source / assets.PROFILE).write_text('changed')
        with self.assertRaisesRegex(RuntimeError, 'joint-profile'): assets.verify_pack(self.unpacked, self.source)

    def test_checksum_and_unsafe_archives_are_rejected_before_extraction(self):
        with self.assertRaisesRegex(RuntimeError, 'checksum'): assets.unpack_verified(self.archive, self.unpacked, b'0' * 64)
        self.assertFalse(self.unpacked.exists())
        for name, kind in (('../escape', tarfile.REGTYPE), ('link', tarfile.SYMTYPE)):
            archive = self.root / 'unsafe.tar.gz'
            with tarfile.open(archive, 'w:gz') as stream:
                info = tarfile.TarInfo(name); info.type = kind; stream.addfile(info)
            with self.assertRaisesRegex(RuntimeError, 'Unsafe'): assets.unpack_verified(archive, self.unpacked)

    def test_corrupt_and_extra_pack_files_are_rejected(self):
        assets.unpack_verified(self.archive, self.unpacked)
        (self.unpacked / 'audio/cues/a.wav').write_bytes(b'bad')
        with self.assertRaisesRegex(RuntimeError, 'checksum'): assets.verify_pack(self.unpacked, self.source)
        (self.unpacked / 'extra').touch()
        with self.assertRaisesRegex(RuntimeError, 'inventory'): assets.verify_pack(self.unpacked, self.source)

    def test_latest_compatible_pack_skips_incompatible_newer_release(self):
        urls = {self.archive.name: 'archive', self.archive.name + '.sha256': 'checksum', self.archive.name + '.manifest.json': 'manifest'}
        good = {'tag_name': 'assets-v1.2.3', 'assets': [{'name': k, 'browser_download_url': v} for k, v in urls.items()]}
        bad = {'tag_name': 'assets-v2.0.0', 'assets': [{'name': 'orion-assets-2.0.0.tar.gz.manifest.json', 'browser_download_url': 'bad-manifest'}]}
        manifest = json.loads((self.root / 'out' / (self.archive.name + '.manifest.json')).read_text())
        responses = {'archive': self.archive.read_bytes(), 'checksum': (self.root / 'out' / (self.archive.name + '.sha256')).read_bytes(),
                     'manifest': json.dumps(manifest).encode(), 'bad-manifest': json.dumps({**manifest, 'joint_profile_sha256': 'wrong'}).encode()}
        with patch.object(assets, 'releases', return_value=iter([bad, good])), patch.object(assets, 'fetch', side_effect=responses.__getitem__):
            self.assertEqual(assets.select_pack(self.source, self.unpacked), self.unpacked)
        with patch.object(assets, 'releases', return_value=iter([bad])), patch.object(assets, 'fetch', side_effect=responses.__getitem__):
            with self.assertRaisesRegex(RuntimeError, 'incompatible'): assets.select_pack(self.source, self.unpacked, 'assets-v2.0.0')

    def test_first_install_only_can_use_bundled_default(self):
        import shutil
        shutil.copy2(self.archive, self.source / 'default-assets.tar.gz')
        with patch.object(assets, 'select_pack', return_value=None):
            self.assertEqual(assets.prepare_catalog(self.source, self.unpacked, first_install=True), self.unpacked)
            with self.assertRaisesRegex(RuntimeError, 'No compatible'): assets.prepare_catalog(self.source, self.root / 'other')


class AssetTransactionTests(Fixture):
    def setUp(self):
        super().setUp()
        self.pose = self.project / 'motion/config/v1/poses.yaml'
        self.motion = self.project / 'motion/motions/v1/nod.yaml'
        self.retired = self.project / 'motion/motions/v1/old.yaml'
        self.scene = self.project / 'scenes/v1/a.yaml'
        for path in (self.pose, self.motion, self.retired, self.scene): self.write(path, 'old YAML\n')
        for path in (self.pose, self.motion, self.scene): self.write(self.release / path.relative_to(self.project), 'new YAML\n')
        self.preserved = [self.project / 'motion/user/poses/v1/a.yaml', self.project / 'scenes/v1/user/a.yaml']
        for path in self.preserved: self.write(path, 'operator content\n')
        self.installed_units()
        self.initial_install = {'version': 2, 'release': str(self.release), 'hardware': 'v1'}
        (self.root / 'installation.json').write_text(json.dumps(self.initial_install))
        inventory = {'project': str(self.project), 'paths': [str(p.relative_to(self.project)) for p in (self.pose, self.motion, self.retired, self.scene)]}
        (self.root / 'catalog-assets.json').write_text(json.dumps(inventory))
        cue = self.release / 'audio/cues/a.wav'; cue.parent.mkdir(parents=True); cue.write_bytes(b'\x00\xff\x80audio')

    def test_assets_update_preserves_stopped_runtime_and_user_files_and_rolls_back_audio(self):
        system = FakeSystem(self.units)
        for state in system.states.values(): state['active'] = False
        old = self.motion.read_bytes()
        receiver.apply_assets(self.root, self.release, system)
        self.assertFalse(any(c[0] in ('start', 'rest') for c in system.calls))
        self.assertEqual((self.project / 'audio/cues/a.wav').read_bytes(), b'\x00\xff\x80audio')
        for path in self.preserved: self.assertEqual(path.read_text(), 'operator content\n')
        receiver.rollback(self.root, system)
        self.assertEqual(self.motion.read_bytes(), old)
        self.assertFalse((self.project / 'audio/cues/a.wav').exists())

    def test_running_runtime_restarts_and_failed_write_restores_catalog(self):
        for failure in (None, 'write'):
            system = FakeSystem(self.units); system.failure = failure
            old = self.motion.read_bytes()
            if failure:
                with self.assertRaisesRegex(RuntimeError, 'Injected'): receiver.apply_assets(self.root, self.release, system)
                self.assertEqual(self.motion.read_bytes(), old)
            else:
                receiver.apply_assets(self.root, self.release, system)
                self.assertIn(('rest',), system.calls); self.assertIn(('start', 'oriond'), system.calls)
                receiver.rollback(self.root, system)
            self.assertFalse((self.root / 'pending-installation.json').exists())

    def test_active_app_change_and_rest_failure_leave_live_catalog_unchanged(self):
        system = FakeSystem(self.units)
        old = self.motion.read_bytes()
        with self.assertRaisesRegex(RuntimeError, 'active app changed'):
            receiver.apply_assets(self.root, self.release, system, expected_release=Path('/other/app'))
        self.assertFalse(system.calls)
        system.failure = 'rest'
        with self.assertRaisesRegex(RuntimeError, 'Unsafe rest'):
            receiver.apply_assets(self.root, self.release, system)
        self.assertEqual(self.motion.read_bytes(), old)
        self.assertNotIn(('stop', 'oriond'), system.calls)
        self.assertTrue(system.state('oriond')['active'])


class ReleasePlanIntegrationTests(unittest.TestCase):
    def test_event_diff_selects_assets_and_reused_tag_is_rejected(self):
        import subprocess
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            def git(*args): return subprocess.check_output(['git', '-C', str(root), *args], text=True).strip()
            git('init', '-q'); git('config', 'user.email', 'test@example.invalid'); git('config', 'user.name', 'Release test')
            (root / 'VERSION').write_text('1.0.0'); (root / 'ASSETS_VERSION').write_text('2.0.0')
            git('add', '.'); git('commit', '-qm', 'Base'); base = git('rev-parse', 'HEAD')
            (root / 'audio/cues').mkdir(parents=True); (root / 'audio/cues/a.wav').write_bytes(b'audio')
            git('add', '.'); git('commit', '-qm', 'Asset update'); head = git('rev-parse', 'HEAD')
            event = root / 'event.json'; event.write_text(json.dumps({'pull_request': {'base': {'sha': base}, 'head': {'sha': head}}}))
            output = root / 'output'
            command = [sys.executable, str(SCRIPTS / 'release_plan.py'), '--event', str(event), '--event-name', 'pull_request', '--output', str(output)]
            subprocess.run(command, cwd=root, check=True, capture_output=True)
            self.assertEqual(output.read_text(), 'kind=assets\ntag=assets-v2.0.0\n')
            git('tag', 'assets-v2.0.0')
            failed = subprocess.run(command, cwd=root, capture_output=True, text=True)
            self.assertNotEqual(failed.returncode, 0); self.assertIn('already exists', failed.stderr)
            event.write_text(json.dumps({'inputs': {'version': 'v1.0.0'}}))
            failed = subprocess.run([*command[:4], '--event-name', 'workflow_dispatch', '--output', str(output)], cwd=root, capture_output=True, text=True)
            self.assertNotEqual(failed.returncode, 0); self.assertIn('prerelease', failed.stderr)

    def test_forced_push_compares_against_the_last_app_release(self):
        import subprocess
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            def git(*args): return subprocess.check_output(['git', '-C', str(root), *args], text=True).strip()
            git('init', '-q'); git('config', 'user.email', 'test@example.invalid'); git('config', 'user.name', 'Release test')
            (root / 'VERSION').write_text('1.0.0'); (root / 'ASSETS_VERSION').write_text('2.0.0')
            git('add', '.'); git('commit', '-qm', 'Base'); git('tag', 'v1.0.0')
            (root / 'audio/cues').mkdir(parents=True); (root / 'audio/cues/a.wav').write_bytes(b'audio')
            git('add', '.'); git('commit', '-qm', 'Asset update')
            (root / 'VERSION').write_text('1.0.1'); git('commit', '-qam', 'App update'); head = git('rev-parse', 'HEAD')
            event = root / 'event.json'; event.write_text(json.dumps({'before': '1' * 40, 'after': head, 'forced': True}))
            output = root / 'output'
            subprocess.run([sys.executable, str(SCRIPTS / 'release_plan.py'), '--event', str(event), '--event-name', 'push',
                            '--output', str(output)], cwd=root, check=True, capture_output=True)
            self.assertEqual(output.read_text(), 'kind=app\ntag=v1.0.1\n')
