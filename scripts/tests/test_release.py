"""Release boundaries: ABI selection, verified payloads, offline environments and commissioning."""
import io
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import Mock, patch
import zipfile

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
import release_install as receiver
import check_hailo
from pi_release_bundle import inspect_archive, pack_bundle, sha256, verify_payload, write_manifest
from pi_release_platform import TARGET_PLATFORM, builder_image, check_platform


class ReleaseTests(unittest.TestCase):
    def test_os_selects_abi_while_shared_binaries_target_bookworm(self):
        pins = json.loads((SCRIPTS / 'release-native.json').read_text())
        for osname in ('bookworm', 'trixie'):
            target = {**TARGET_PLATFORM, 'version_codename': osname, 'system_python': '3.11' if osname == 'bookworm' else '3.13'}
            self.assertEqual(builder_image(target), 'debian:bookworm')
            check_platform(TARGET_PLATFORM, {**target, 'raspberry_pi_os': True})
            packages = receiver.system_packages(osname, True, pins[osname])
            self.assertFalse(any('tappas' in p or 'python3-hailort' in p for p in packages))
            self.assertTrue(any(p.startswith('hailort=') for p in packages))
            self.assertIn('linux-image-rpi-2712', packages)
            self.assertIn('linux-headers-rpi-2712', packages)
            no_hat = receiver.system_packages(osname, False, pins[osname])
            self.assertFalse(any(p.startswith('hailo') for p in no_hat))
        with self.assertRaises(RuntimeError): builder_image({**TARGET_PLATFORM, 'machine': 'x86_64'})
        with self.assertRaises(RuntimeError): check_platform(TARGET_PLATFORM, {**TARGET_PLATFORM, 'raspberry_pi_os': False})
        with self.assertRaises(RuntimeError): builder_image({**TARGET_PLATFORM, 'version_codename': 'jammy'})

    def test_manifest_inventory_and_corruption(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder); release = root / 'release'; release.mkdir()
            (release / 'release.json').write_text(json.dumps({'release_id': 'a'*12+'-12345678', 'commit': 'a'*40, 'hardware': 'v2'}))
            (release / 'example').write_bytes(b'payload')
            manifest = write_manifest(root, release, TARGET_PLATFORM)
            (root / 'SHA256SUMS').write_text('external inventory')
            verify_payload(root, manifest)
            (release / 'example').write_bytes(b'corrupt')
            with self.assertRaisesRegex(RuntimeError, 'checksum'): verify_payload(root, manifest)
            (release / 'extra').touch()
            with self.assertRaisesRegex(RuntimeError, 'inventory'): verify_payload(root, manifest)

    def test_archive_rejects_traversal_and_links_before_extraction(self):
        for name, kind in [('../escape', tarfile.REGTYPE), ('/escape', tarfile.REGTYPE), ('link', tarfile.SYMTYPE), ('device', tarfile.CHRTYPE)]:
            with self.subTest(name=name), tempfile.TemporaryFile() as stream:
                with tarfile.open(fileobj=stream, mode='w') as archive:
                    entry = tarfile.TarInfo(name); entry.type = kind; archive.addfile(entry)
                stream.seek(0)
                with tarfile.open(fileobj=stream) as archive, self.assertRaises(RuntimeError): inspect_archive(archive)

    def test_every_environment_uses_bundled_python_and_offline_wheels(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for component in receiver.COMPONENTS:
                wheels = root / 'wheels' / component; wheels.mkdir(parents=True)
                with zipfile.ZipFile(wheels / f'{component}-1-py3-none-any.whl', 'w') as wheel: wheel.writestr('module.py', '')
            native = root / 'wheels/native/bookworm'; native.mkdir(parents=True)
            for name in ('rpi_libcamera', 'rpi_kms', 'hailort'): (native / f'{name}-1-cp312-linux_aarch64.whl').touch()
            with patch.object(receiver, 'run') as run:
                receiver.prepare_environments(root / 'release', root / 'wheels', Path('/bundle/uv'), 'bookworm', False)
            commands = [tuple(map(str, call.args)) for call in run.call_args_list]
            venvs = [c for c in commands if 'venv' in c]
            self.assertEqual(len(venvs), 5)
            for c in venvs:
                self.assertIn(str(receiver.PYTHON), c); self.assertIn('--offline', c)
                self.assertNotIn('--system-site-packages', c); self.assertNotIn('/usr/bin/python3', c)
            native_install = next(c for c in commands if any('rpi_libcamera-' in p for p in c))
            self.assertFalse(any('hailort-' in p for p in native_install))
            for flag in ('--offline', '--no-index', '--no-build', '--no-deps'): self.assertIn(flag, native_install)

    def test_hailo_identification_distinguishes_8_and_8l(self):
        self.assertEqual(check_hailo.device_kind('Device Architecture: HAILO8L'), 'HAILO8L')
        self.assertEqual(check_hailo.device_kind('Device Architecture: Hailo-8'), 'HAILO8')
        with self.assertRaises(RuntimeError): check_hailo.device_kind('HAILO10H')
        self.assertTrue(all(len(digest) == 64 for _, digest in check_hailo.MODELS.values()))

    def test_no_hat_does_not_touch_boot_configuration(self):
        with patch.object(receiver, 'run') as run:
            receiver.configure_hailo(False, Path('/does/not/exist'))
        run.assert_not_called()

    def test_failed_python_copy_is_removed_before_retry(self):
        with tempfile.TemporaryDirectory() as folder:
            destination = Path(folder) / 'python'
            temporary = Path(folder) / 'python.temporary'
            def execute(*args, **kwargs):
                if 'mktemp' in args: return subprocess.CompletedProcess(args, 0, str(temporary) + '\n')
                if 'cp' in args: raise subprocess.CalledProcessError(1, args)
                return subprocess.CompletedProcess(args, 0)
            with patch.object(receiver, 'PYTHON', destination / 'bin/python3.12'), patch.object(receiver, 'run', side_effect=execute) as run:
                with self.assertRaises(subprocess.CalledProcessError): receiver.prepare_python(Path('/bundle/python'))
            self.assertEqual(run.call_args.args, ('sudo', 'rm', '-rf', temporary))

    def test_hailo_gen3_is_global_and_idempotent(self):
        with tempfile.TemporaryDirectory() as folder:
            boot = Path(folder) / 'config.txt'
            boot.write_text('[pi4]\ndtparam=pciex1_gen=3\n')
            with patch.object(receiver, 'run') as run:
                receiver.configure_hailo(True, boot)
            self.assertEqual(run.call_args.args, ('sudo', 'tee', '-a', boot))
            self.assertIn('[all]\n', run.call_args.kwargs['input'])
            boot.write_text(boot.read_text() + run.call_args.kwargs['input'])
            with patch.object(receiver, 'run') as run:
                receiver.configure_hailo(True, boot)
            run.assert_not_called()

    def test_unexpected_python_directory_is_preserved(self):
        with tempfile.TemporaryDirectory() as folder:
            with patch.object(receiver, 'PYTHON', Path(folder) / 'bin/python3.12'), patch.object(receiver, 'run') as run:
                with self.assertRaisesRegex(RuntimeError, 'Unexpected Python directory'): receiver.prepare_python(Path('/bundle/python'))
            run.assert_not_called()


if __name__ == '__main__': unittest.main()
