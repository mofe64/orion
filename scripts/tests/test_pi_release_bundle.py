"""The Pi may install wheels but must never build code or retain installation caches."""
import io
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import zipfile

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
import deploy_pi_release as receiver
from pi_release_bundle import inspect_archive, pack_bundle, sha256, write_manifest
from pi_release_platform import TARGET_PLATFORM, builder_image


class BundleTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.root = self.base / 'stack'
        self.platform = patch.object(receiver, 'read_platform', return_value=TARGET_PLATFORM)
        self.platform.start(); self.addCleanup(self.platform.stop)
        self.assets = patch.object(receiver, 'prepare_assets').start()
        self.voice = patch.object(receiver, 'validate_voice').start()
        self.vision = patch.object(receiver, 'prepare_vision', create=True).start()
        self.run = patch.object(receiver, 'run').start()
        self.addCleanup(patch.stopall)

    def bundle(self, suffix='12345678', hardware='v2'):
        folder = self.base / f'bundle-{suffix}'; folder.mkdir()
        release = folder / 'release'; release.mkdir()
        shutil.copytree(SCRIPTS.parent / 'hardware/profiles', release / 'hardware/profiles')
        (release / 'release.json').write_text(json.dumps({
            'revision': 'a' * 12, 'commit': 'a' * 40, 'release_id': 'a' * 12 + '-' + suffix, 'hardware': hardware}))
        for component in ('voice', 'speech', 'vision'):
            (release / component).mkdir()
            wheels = folder / 'wheels' / component; wheels.mkdir(parents=True)
            with zipfile.ZipFile(wheels / f'{component}-0.1-py3-none-any.whl', 'w') as wheel:
                wheel.writestr(f'{component}.py', '# prepared runtime')
        write_manifest(folder, release, TARGET_PLATFORM)
        destination = self.base / f'{suffix}.tar.gz'
        pack_bundle(folder, destination)
        return destination, folder

    def prepare(self, bundle, hardware='v2'):
        return receiver.prepare_bundle(bundle, sha256(bundle), self.root, hardware)

    def test_two_preparations_leave_only_releases_and_no_build_wheel_or_package_cache(self):
        for suffix in ('12345678', '87654321'):
            bundle, _ = self.bundle(suffix)
            release = self.prepare(bundle)
            self.assertTrue((release / 'release.json').exists())
        self.assertEqual(len(list((self.root / 'releases').iterdir())), 2)
        self.assertEqual(sorted(path.name for path in self.root.iterdir()), ['deployment.lock', 'releases'])
        self.assertFalse(list(self.root.rglob('*.whl')))
        commands = [tuple(map(str, call.args)) for call in self.run.call_args_list]
        self.assertFalse(any(command[0] in ('cargo', 'rustc', 'cc', 'gcc', 'pip') for command in commands))
        installs = [command for command in commands if command[:3] == ('uv', 'pip', 'install')]
        self.assertEqual(len(installs), 6)
        for command in installs:
            for flag in ('--offline', '--no-index', '--no-build', '--no-deps', '--no-cache', '--no-config'):
                self.assertIn(flag, command)
        venvs = [command for command in commands if command[:2] == ('uv', 'venv')]
        self.assertTrue(all('--no-python-downloads' in command and '--no-cache' in command for command in venvs))
        vision = next(command for command in venvs if command[-1].endswith('vision/.venv'))
        self.assertIn('--system-site-packages', vision)
        self.assertIn('/usr/bin/python3', vision)
        speech = next(command for command in venvs if command[-1].endswith('speech/.venv'))
        self.assertIn('3.11', speech)

    def test_damaged_upload_is_rejected_before_creating_a_release(self):
        bundle, _ = self.bundle()
        with self.assertRaisesRegex(RuntimeError, 'checksum'):
            receiver.prepare_bundle(bundle, '0' * 64, self.root, 'v2')
        self.run.assert_not_called()
        self.assertFalse(self.root.exists())

    def test_vision_asset_is_prepared_only_for_camera_releases(self):
        for hardware in ('v1', 'v2'):
            bundle, _ = self.bundle(suffix='12345678' if hardware == 'v1' else '87654321', hardware=hardware)
            release = self.prepare(bundle, hardware)
            if hardware == 'v2':
                self.vision.assert_called_once_with(release)
            else:
                self.vision.assert_not_called()

    def test_detector_preparation_failure_removes_new_release_before_activation(self):
        bundle, _ = self.bundle()
        self.vision.side_effect = RuntimeError('YuNet checksum mismatch')
        with self.assertRaisesRegex(RuntimeError, 'checksum mismatch'):
            self.prepare(bundle)
        self.assertEqual(list((self.root / 'releases').iterdir()), [])
        self.voice.assert_not_called()

    def test_platform_change_is_rejected_before_installing_or_creating_a_release(self):
        bundle, _ = self.bundle()
        with patch.object(receiver, 'read_platform', return_value={**TARGET_PLATFORM, 'version_id': '26.04'}):
            with self.assertRaisesRegex(RuntimeError, 'platform changed'):
                self.prepare(bundle)
        self.run.assert_not_called()
        self.assertFalse(self.root.exists())

    def test_wrong_hardware_is_rejected_before_installation(self):
        bundle, _ = self.bundle()
        with self.assertRaisesRegex(RuntimeError, 'hardware selection'):
            self.prepare(bundle, 'v1')
        self.run.assert_not_called()

    def test_insufficient_space_is_rejected_before_unpacking(self):
        bundle, _ = self.bundle()
        with patch.object(receiver.shutil, 'disk_usage', return_value=SimpleNamespace(free=1)):
            with self.assertRaisesRegex(RuntimeError, 'unpacking and installing'):
                self.prepare(bundle)
        self.run.assert_not_called()
        self.assertFalse((self.root / 'releases').exists())
        self.assertFalse(list(self.root.glob('.incoming-*')))

    def test_payload_damage_is_rejected_before_installing(self):
        bundle, folder = self.bundle()
        (folder / 'release/release.json').write_text('changed')
        pack_bundle(folder, bundle)
        with self.assertRaisesRegex(RuntimeError, 'checksum mismatch'):
            self.prepare(bundle)
        self.run.assert_not_called()
        self.assertFalse((self.root / 'releases').exists())
        self.assertFalse(list(self.root.glob('.incoming-*')))

    def test_missing_wheels_never_fall_back_to_source_builds_and_remove_failed_release(self):
        bundle, folder = self.bundle()
        for wheel in (folder / 'wheels/voice').glob('*.whl'): wheel.unlink()
        (folder / 'bundle.json').unlink()
        write_manifest(folder, folder / 'release', TARGET_PLATFORM)
        pack_bundle(folder, bundle)
        with self.assertRaisesRegex(RuntimeError, 'No prepared wheels'):
            self.prepare(bundle)
        self.assertEqual(list((self.root / 'releases').iterdir()), [])
        self.assertFalse(list(self.root.glob('.incoming-*')))
        self.assertEqual(self.run.call_count, 1)  # Creating a venv is the only attempted operation.

    def test_install_failure_or_cancellation_removes_only_new_release(self):
        self.root.mkdir()
        active = self.root / 'releases/active'; active.mkdir(parents=True)
        (active / 'keep').write_text('running release')
        for suffix, error in [('12345678', subprocess.CalledProcessError(1, ['uv'])), ('87654321', KeyboardInterrupt())]:
            bundle, _ = self.bundle(suffix)
            self.run.side_effect = error
            with self.assertRaises(type(error)):
                self.prepare(bundle)
            self.assertEqual(list((self.root / 'releases').iterdir()), [active])
            self.assertFalse(list(self.root.glob('.incoming-*')))
            self.assertEqual((active / 'keep').read_text(), 'running release')

    def test_interrupted_activation_blocks_preparation(self):
        bundle, _ = self.bundle()
        self.root.mkdir()
        (self.root / 'pending-installation.json').write_text('{}')
        with self.assertRaisesRegex(RuntimeError, 'interrupted deployment'):
            self.prepare(bundle)
        self.run.assert_not_called()

    def test_reusing_release_identity_does_not_overwrite_existing_files(self):
        bundle, _ = self.bundle()
        release = self.prepare(bundle)
        (release / 'operator-marker').write_text('keep')
        self.run.reset_mock()
        with self.assertRaisesRegex(RuntimeError, 'already exists'):
            self.prepare(bundle)
        self.run.assert_not_called()
        self.assertEqual((release / 'operator-marker').read_text(), 'keep')

    def test_archive_rejects_traversal_symlinks_and_duplicate_names(self):
        for name, kind in [('../escape', tarfile.REGTYPE), ('release/link', tarfile.SYMTYPE), ('duplicate', tarfile.REGTYPE)]:
            with self.subTest(name=name):
                stream = io.BytesIO()
                with tarfile.open(fileobj=stream, mode='w') as archive:
                    member = tarfile.TarInfo(name); member.type = kind
                    archive.addfile(member)
                    if name == 'duplicate': archive.addfile(member)
                stream.seek(0)
                with tarfile.open(fileobj=stream) as archive:
                    with self.assertRaises(RuntimeError): inspect_archive(archive)

    def test_builder_refuses_a_different_robot_environment(self):
        self.assertEqual(builder_image(TARGET_PLATFORM), 'ubuntu:24.04')
        for key, value in [('id', 'debian'), ('machine', 'x86_64'), ('system_python', '3.13'), ('libc', 'glibc-2.40')]:
            with self.subTest(key=key), self.assertRaisesRegex(RuntimeError, 'targets Ubuntu'):
                builder_image({**TARGET_PLATFORM, key: value})


class VisionAssetTests(unittest.TestCase):
    def test_deployment_uses_the_detectors_pinned_checksum(self):
        import ast
        source = ast.parse((SCRIPTS.parent / 'vision/orion_vision/detect_faces.py').read_text())
        checksum = next(ast.literal_eval(node.value) for node in source.body if isinstance(node, ast.Assign)
                        and any(isinstance(target, ast.Name) and target.id == 'MODEL_SHA256' for target in node.targets))
        self.assertEqual(receiver.YUNET_SHA256, checksum)

    def test_model_download_is_pinned_verified_and_reused(self):
        import prepare_pi_voice_stack as assets
        with tempfile.TemporaryDirectory() as directory:
            release = Path(directory)
            payload = b'synthetic model fixture'
            with patch.object(receiver, 'YUNET_SHA256', hashlib.sha256(payload).hexdigest()), \
                 patch.object(assets.urllib.request, 'urlopen', return_value=io.BytesIO(payload)) as fetch:
                receiver.prepare_vision(release)
                receiver.prepare_vision(release)
            self.assertEqual((release / 'vision/models/face_detection_yunet_2023mar.onnx').read_bytes(), payload)
            fetch.assert_called_once_with(receiver.YUNET_URL, timeout=120)
            self.assertIn('47534e27c9851bb1128ccc0102f1145e27f23f98', receiver.YUNET_URL)

    def test_corrupt_model_is_never_installed_and_temporary_download_is_removed(self):
        import prepare_pi_voice_stack as assets
        with tempfile.TemporaryDirectory() as directory:
            release = Path(directory)
            with patch.object(assets.urllib.request, 'urlopen', return_value=io.BytesIO(b'corrupt model')):
                with self.assertRaisesRegex(RuntimeError, 'Checksum mismatch'):
                    receiver.prepare_vision(release)
            self.assertFalse(list(release.rglob('*.onnx')))
            self.assertFalse(list(release.rglob('*.download')))


if __name__ == '__main__':
    unittest.main()
