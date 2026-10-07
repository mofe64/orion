"""Built-in YAML deployment, ownership and rollback with preserved operator assets."""
import json
from pathlib import Path
import subprocess
import sys
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
from pi_catalog import catalog_plan
import install_pi_voice_stack as installer
from test_pi_release_install import Fixture, FakeSystem


class CatalogTests(Fixture):
    def setUp(self):
        super().setUp()
        self.pose = self.project / 'motion/config/v1/poses.yaml'
        self.motion = self.project / 'motion/motions/v1/idle/idle_breathe.yaml'
        self.retired = self.project / 'motion/motions/v1/idle/retired.yaml'
        self.scene = self.project / 'scenes/v1/deployment_smoke.yaml'
        for path in (self.pose, self.motion, self.retired, self.scene):
            self.write(path, 'old YAML\n')
        self.git('init', '-q')
        self.git('config', 'user.email', 'test@example.invalid')
        self.git('config', 'user.name', 'Catalog test')
        self.git('add', '.'); self.git('commit', '-qm', 'Existing built-ins')
        self.write(self.motion, 'local edit backed up during activation\n')
        self.preserved = [self.project / 'motion/user/poses/v1/custom.yaml',
                          self.project / 'motion/motions/v1/user/custom.yaml',
                          self.project / 'scenes/v1/user/custom.yaml',
                          self.project / 'motion/motions/v1/local.yaml',
                          self.home / '.config/orion/servo_calibration.json']
        for path in self.preserved:
            self.write(path, 'operator content\n')
        for path in (self.pose, self.motion, self.scene):
            self.write(self.release / path.relative_to(self.project), 'release YAML\n')
        for relative in ('motion/motions/v1/user/unwanted.yaml', 'scenes/v1/user/unwanted.yaml'):
            self.write(self.release / relative, 'do not deploy user assets\n')
        self.added = self.project / 'motion/motions/v1/idle/added.yml'
        self.write(self.release / self.added.relative_to(self.project), 'added YAML\n')

    def git(self, *args):
        return subprocess.run(['git', '-C', str(self.project), *args],
                              check=True, capture_output=True, text=True)

    def assets(self):
        return catalog_plan(self.release, self.project, self.root)

    def test_first_update_replaces_builtins_removes_retired_and_preserves_operator_files(self):
        head = self.git('rev-parse', 'HEAD').stdout
        status = self.git('status', '--porcelain').stdout
        assets = self.assets()
        for path in (self.pose, self.motion, self.scene):
            self.assertEqual(assets[path], 'release YAML\n')
        self.assertEqual(assets[self.added], 'added YAML\n')
        self.assertIsNone(assets[self.retired])
        self.assertTrue(all(p not in assets for p in self.preserved))
        self.assertFalse(any('/user/' in str(p.relative_to(self.project))
                             for p in assets if p.is_relative_to(self.project)))
        self.assertEqual(self.git('rev-parse', 'HEAD').stdout, head)
        self.assertEqual(self.git('status', '--porcelain').stdout, status)
        self.assertEqual(self.motion.read_text(), 'local edit backed up during activation\n')

    def test_inventory_tracks_additions_for_removal_on_the_next_release(self):
        assets = self.assets()
        manifest = self.root / 'catalog-assets.json'
        manifest.write_text(assets[manifest])
        (self.release / self.added.relative_to(self.project)).unlink()
        self.assertIsNone(self.assets()[self.added])

    def test_previous_unversioned_builtin_inventory_is_retired(self):
        old_paths = ['motion/config/poses.yaml', 'motion/motions/functional/return_home.yaml',
                     'scenes/return_home.yaml', 'hardware/v2/poses.yaml']
        manifest = self.root / 'catalog-assets.json'
        manifest.write_text(json.dumps({'project': str(self.project), 'paths': old_paths}))
        for relative in old_paths:
            self.write(self.project / relative, 'old built-in\n')
        assets = self.assets()
        for relative in old_paths:
            self.assertIsNone(assets[self.project / relative])
        self.assertEqual(assets[self.pose], 'release YAML\n')
        self.assertFalse(set(old_paths) & set(json.loads(assets[manifest])['paths']))

    def test_both_versions_are_deployed_without_publishing_user_assets(self):
        for relative in ('motion/config/v2/poses.yaml', 'motion/motions/v2/nod.yaml',
                         'scenes/v2/return_home.yaml'):
            self.write(self.release / relative, 'V2 YAML\n')
            self.assertEqual(self.assets()[self.project / relative], 'V2 YAML\n')
        for relative in ('motion/motions/v2/user/custom.yaml', 'scenes/v2/user/custom.yaml'):
            self.write(self.release / relative, 'user YAML\n')
            self.assertNotIn(self.project / relative, self.assets())

    def test_legacy_user_assets_move_to_their_version_and_rollback_restores_them(self):
        moves = (('motion/user/poses/custom.yaml', 'motion/user/poses/v1/custom.yaml'),
                 ('motion/motions/user/custom.yaml', 'motion/motions/v1/user/custom.yaml'),
                 ('scenes/user/custom.yaml', 'scenes/v1/user/custom.yaml'),
                 ('hardware/v2/user/poses/custom.yaml', 'motion/user/poses/v2/custom.yaml'),
                 ('hardware/v2/motions/user/custom.yaml', 'motion/motions/v2/user/custom.yaml'),
                 ('hardware/v2/scenes/user/custom.yaml', 'scenes/v2/user/custom.yaml'))
        for old, new in moves:
            self.write(self.project / old, 'operator content\n')
            assets = self.assets()
            self.assertIsNone(assets[self.project / old])
            self.assertEqual(assets[self.project / new], 'operator content\n')
            self.assertTrue((self.project / old).exists())
        self.installed_units()
        system = FakeSystem(self.units)
        installer.activate(self.root, self.release, self.home,
                           {**self.plan(), **self.assets()}, system)
        for old, new in moves:
            self.assertFalse((self.project / old).exists())
            self.assertEqual((self.project / new).read_text(), 'operator content\n')
        installer.rollback(self.root, system)
        for old, _ in moves:
            self.assertEqual((self.project / old).read_text(), 'operator content\n')

    def test_user_asset_move_rejects_conflicting_destination(self):
        self.write(self.project / 'scenes/user/custom.yaml', 'different legacy content\n')
        with self.assertRaisesRegex(ValueError, 'already exists at versioned location'):
            self.assets()

    def test_invalid_inventory_and_symlinked_destinations_are_rejected(self):
        manifest = self.root / 'catalog-assets.json'
        for relative in ('../escape.yaml', 'motion/motions/v1/user/custom.yaml', '/tmp/outside.yaml'):
            manifest.write_text(json.dumps({'project': str(self.project), 'paths': [relative]}))
            with self.assertRaisesRegex(ValueError, 'Invalid built-in catalog inventory'):
                self.assets()
        manifest.unlink()
        self.motion.unlink(); self.motion.symlink_to(self.preserved[0])
        with self.assertRaisesRegex(ValueError, 'symlinked catalog asset'):
            self.assets()
        self.assertEqual(self.preserved[0].read_text(), 'operator content\n')

    def test_validation_uses_the_planned_motion_catalog_without_changing_live_files(self):
        class Compiler:
            def __init__(self): self.calls = []
            def run(inner, *args, **kwargs):
                inner.calls.append(args)
                poses = Path(args[args.index('--pose-file') + 1])
                motions = Path(args[args.index('--motions-directory') + 1])
                self.assertEqual(poses.read_text(), 'release YAML\n')
                self.assertEqual((motions / 'idle/idle_breathe.yaml').read_text(), 'release YAML\n')
                self.assertFalse((motions / 'idle/retired.yaml').exists())
                self.assertEqual((motions / 'user/custom.yaml').read_text(), 'operator content\n')
        compiler = Compiler()
        installer.validate_catalog(self.release, self.project, self.preserved[-1], compiler, self.assets())
        self.assertEqual([call[call.index("--motion") + 1] for call in compiler.calls],
                         ['look_at_left_expressive', 'look_at_right_expressive'])
        self.assertEqual(self.motion.read_text(), 'local edit backed up during activation\n')
        self.assertTrue(self.retired.exists())
        self.assertFalse(self.added.exists())

    def transaction(self, failure=None):
        self.installed_units()
        system = FakeSystem(self.units)
        system.failure = failure
        files = {**self.plan(), **self.assets()}
        head = self.git('rev-parse', 'HEAD').stdout
        before = {p: p.read_bytes() for p in (self.pose, self.motion, self.retired, self.scene, *self.preserved)}
        if failure:
            with self.assertRaisesRegex(RuntimeError, 'Injected'):
                installer.activate(self.root, self.release, self.home, files, system)
        else:
            installer.activate(self.root, self.release, self.home, files, system)
            self.assertEqual(self.motion.read_text(), 'release YAML\n')
            self.assertFalse(self.retired.exists())
            self.assertTrue(self.added.exists())
            calls = system.calls
            self.assertLess(calls.index(('rest',)), calls.index(('write', str(self.motion))))
            self.assertLess(calls.index(('stop', 'oriond')), calls.index(('write', str(self.motion))))
            self.assertLess(calls.index(('write', str(self.motion))), calls.index(('smoke',)))
            installer.rollback(self.root, system)
        for path, data in before.items():
            self.assertEqual(path.read_bytes(), data)
        self.assertFalse(self.added.exists())
        self.assertFalse((self.root / 'catalog-assets.json').exists())
        self.assertEqual(self.git('rev-parse', 'HEAD').stdout, head)
        self.assertFalse((self.root / 'pending-installation.json').exists())

    def test_successful_update_and_manual_rollback_restore_the_previous_yaml(self):
        self.transaction()

    def test_smoke_or_readiness_failure_restores_yaml_without_touching_user_assets(self):
        for failure in ('smoke', 'ready'):
            with self.subTest(failure=failure):
                self.transaction(failure)


if __name__ == '__main__':
    unittest.main()
