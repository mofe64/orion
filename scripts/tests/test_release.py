"""Release boundaries: ABI selection, verified payloads, offline environments and commissioning."""
import io
import gzip
import json
import os
from pathlib import Path
import shutil
import shlex
import subprocess
import sys
import tarfile
import tempfile
import textwrap
import unittest
from unittest.mock import Mock, patch
import zipfile

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
import release_install as receiver
import check_hailo
import native_packages
from pi_release_bundle import inspect_archive, pack_bundle, sha256, verify_payload, write_manifest
from pi_release_platform import TARGET_PLATFORM, builder_image, check_platform


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.pins = json.loads((SCRIPTS / 'release-native.json').read_text())

    def available(self, pins, hailo=True):
        return '\n'.join(f'{p.split("=", 1)[0]} | {p.split("=", 1)[1]} | archive' for p in native_packages.pinned_packages(pins, hailo))

    def test_native_packages_are_held_after_install_on_both_os_versions(self):
        for codename in ('bookworm', 'trixie'):
            with self.subTest(codename=codename), tempfile.TemporaryDirectory() as folder:
                pins = self.pins[codename]
                with patch.object(receiver, 'run', return_value=subprocess.CompletedProcess([], 0, self.available(pins))) as run:
                    receiver.install_system_packages(Path(folder), codename, True, pins)
                commands = [call.args for call in run.call_args_list]
                install = next(c for c in commands if c[:3] == ('sudo', 'apt-get', 'install'))
                self.assertIn('--allow-change-held-packages', install)
                self.assertIn('--allow-downgrades', install)
                expected = tuple(p.split('=', 1)[0] for p in native_packages.pinned_packages(pins))
                self.assertEqual(commands[-1], ('sudo', 'apt-mark', 'hold', *expected))
                self.assertEqual(json.loads((Path(folder) / 'native-packages.json').read_text())['packages'], list(expected))

    def test_update_replaces_only_orion_managed_holds(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / 'native-packages.json').write_text(json.dumps({'packages': ['libcamera-old', 'libcamera-ipa']}))
            pins = self.pins['bookworm']
            with patch.object(receiver, 'run', return_value=subprocess.CompletedProcess([], 0, self.available(pins, False))) as run:
                receiver.install_system_packages(root, 'bookworm', False, pins)
            commands = [call.args for call in run.call_args_list]
            self.assertIn(('sudo', 'apt-mark', 'unhold', 'libcamera-old'), commands)
            self.assertFalse(any('hailort' in c for c in commands))

    def test_missing_pin_stops_before_install_or_hold_changes(self):
        with tempfile.TemporaryDirectory() as folder, patch.object(receiver, 'run', return_value=subprocess.CompletedProcess([], 0, '')) as run:
            with self.assertRaisesRegex(RuntimeError, 'no longer in the Raspberry Pi archive') as error:
                receiver.install_system_packages(Path(folder), 'bookworm', True, self.pins['bookworm'])
            self.assertEqual(str(error.exception), native_packages.UNAVAILABLE)
            self.assertFalse(any('install' in call.args or 'apt-mark' in call.args for call in run.call_args_list))

    def test_rollback_restores_previous_release_package_pins(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder); old = root / 'releases/old'; (old / 'scripts').mkdir(parents=True)
            pins = json.loads(json.dumps(self.pins)); pins['bookworm']['libcamera_version'] = 'older-version'
            (old / 'scripts/release-native.json').write_text(json.dumps(pins))
            backup = root / 'backup'; backup.write_text(json.dumps({'release': str(old)}))
            data = {'files': [{'path': str(root / 'installation.json'), 'backup': str(backup)}]}
            with patch.object(receiver, 'install_system_packages') as install, patch.object(receiver, 'has_hailo', return_value=True):
                receiver.restore_native_packages(root, 'bookworm', root, data)
            install.assert_called_once_with(root, 'bookworm', True, pins['bookworm'])

    def test_checks_show_services_and_hailo_even_when_servo_check_fails(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder); release = root / 'release'
            (root / 'installation.json').write_text(json.dumps({'release': str(release)}))
            failure = subprocess.CalledProcessError(1, ['oriond', '--check'])
            with patch.object(receiver, 'run', side_effect=[failure, subprocess.CompletedProcess([], 0), subprocess.CompletedProcess([], 0)]) as run:
                with self.assertRaisesRegex(RuntimeError, 'Servo positions'):
                    receiver.check_installation(root)
            commands = [call.args for call in run.call_args_list]
            self.assertEqual(commands[0], (release / 'runtime/target/release/oriond', '--check'))
            self.assertIn('status', commands[1])
            self.assertTrue(set(receiver.MANAGED_SERVICES).issubset(commands[1]))
            self.assertEqual(commands[2][1], release / 'scripts/check_hailo.py')

    def test_archive_preflight_covers_build_and_runtime_pins(self):
        def fetch(url):
            codename = 'bookworm' if '/bookworm/' in url else 'trixie'
            paragraphs = [f'Package: {p.split("=", 1)[0]}\nVersion: {p.split("=", 1)[1]}'
                          for p in native_packages.pinned_packages(self.pins[codename], build=True)]
            return gzip.compress('\n\n'.join(paragraphs).encode())
        native_packages.check_archives(self.pins, fetch)
        for codename in ('bookworm', 'trixie'):
            with self.subTest(codename=codename):
                changed = json.loads(json.dumps(self.pins)); changed[codename]['hailo_packages'] += ['missing=1']
                with self.assertRaisesRegex(RuntimeError, codename + ': missing=1'):
                    native_packages.check_archives(changed, fetch)

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

    def test_bootstrap_entry_point_keeps_extracted_payload_unchanged_before_verification(self):
        with tempfile.TemporaryDirectory() as folder:
            base = Path(folder); source = base / 'source'
            release = source / 'release'; scripts = release / 'scripts'
            scripts.mkdir(parents=True)
            for module in SCRIPTS.glob('*.py'):
                shutil.copy(module, scripts / module.name)
            (release / 'release.json').write_text(json.dumps({
                'release_id': 'a'*12+'-12345678', 'commit': 'a'*40, 'hardware': 'v2'}))
            bootstrap = source / 'python/bin/python3.12'
            bootstrap.parent.mkdir(parents=True)
            # Use the host interpreter behind the bundle path in this portable fixture.
            bootstrap.write_text('#!/bin/sh\nexec ' + shlex.quote(sys.executable) + ' "$@"\n')
            bootstrap.chmod(0o755)
            manifest = write_manifest(source, release, TARGET_PLATFORM)
            bundle = base / 'bundle.tar.gz'; pack_bundle(source, bundle)
            payload = base / 'stage/payload'; payload.mkdir(parents=True)
            with tarfile.open(bundle) as archive:
                inspect_archive(archive)
                archive.extractall(payload, filter='data')
            def files():
                return sorted(p.relative_to(payload).as_posix() for p in payload.rglob('*') if p.is_file())
            expected = files(); inventory = base / 'expected.json'
            inventory.write_text(json.dumps(expected))
            hooks = base / 'hooks'; hooks.mkdir()
            # Instrument the real verification boundary, then stop before Pi/system changes.
            (hooks / 'sitecustomize.py').write_text(textwrap.dedent("""\
                import json, os, sys
                from pathlib import Path
                import pi_release_bundle
                original_verify = pi_release_bundle.verify_payload
                def checked_verify(payload, manifest):
                    expected = json.loads(Path(os.environ['ORION_TEST_INVENTORY']).read_text())
                    actual = sorted(p.relative_to(payload).as_posix() for p in payload.rglob('*') if p.is_file())
                    assert actual == expected, 'Bootstrap changed payload inventory before verify_payload'
                    assert '-B' in sys.orig_argv, 'Bootstrap must pass -B'
                    assert os.environ.get('PYTHONDONTWRITEBYTECODE') == '1'
                    original_verify(payload, manifest)
                    print('Extracted payload unchanged and verified')
                    raise SystemExit(0)
                pi_release_bundle.verify_payload = checked_verify
                """))
            environment = os.environ.copy()
            for name in ('PYTHONDONTWRITEBYTECODE', 'PYTHONPYCACHEPREFIX'):
                environment.pop(name, None)
            environment.update(stage=str(payload.parent), root=str(base / 'installed'),
                VERSION_CODENAME='bookworm', ORION_TEST_INVENTORY=str(inventory),
                PYTHONPATH=os.pathsep.join((str(hooks), str(payload / 'release/scripts'))))
            installer = (SCRIPTS / 'install.sh').read_text()
            command = installer[installer.index('bootstrap=$stage/payload/python/bin/python3.12\n'):]
            result = subprocess.run(['bash', '-euo', 'pipefail', '-c', command], env=environment,
                                    capture_output=True, text=True, timeout=20)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn('Extracted payload unchanged and verified', result.stdout)
            self.assertEqual(files(), expected)
            verify_payload(payload, manifest)

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

    def test_create_token_prints_only_its_private_file_path(self):
        with tempfile.TemporaryDirectory() as folder:
            token = Path(folder) / '.config/orion/studio-token'
            command = [sys.executable, str(SCRIPTS.parent / 'orion_studio/gateway.py'),
                       'create-token', '--token-file', str(token)]
            result = subprocess.run(command, capture_output=True, text=True, check=True, timeout=20)
            secret = token.read_text().strip()
            self.assertGreaterEqual(len(secret), 32)
            self.assertEqual(token.stat().st_mode & 0o777, 0o600)
            self.assertNotIn(secret, result.stdout + result.stderr)
            self.assertIn(str(token), result.stdout)
            # Creating a token must never replace an existing pairing secret.
            repeated = subprocess.run(command, capture_output=True, text=True, timeout=20)
            self.assertNotEqual(repeated.returncode, 0)
            self.assertEqual(token.read_text().strip(), secret)
            self.assertNotIn(secret, repeated.stdout + repeated.stderr)

    def test_hailo_identification_distinguishes_8_and_8l(self):
        self.assertEqual(check_hailo.device_kind('Device Architecture: HAILO8'), 'HAILO8')
        self.assertEqual(check_hailo.device_kind('Device Architecture: HAILO8L'), 'HAILO8L')
        self.assertEqual(check_hailo.device_kind('Device Architecture: Hailo-8'), 'HAILO8')
        with self.assertRaises(RuntimeError): check_hailo.device_kind('HAILO10H')
        self.assertTrue(all(len(digest) == 64 for _, digest in check_hailo.MODELS.values()))

    def test_hailo_architecture_overrides_the_generic_board_name(self):
        output = 'Board Name: Hailo-8\nDevice Architecture: HAILO8L\n'
        self.assertEqual(check_hailo.device_kind(output), 'HAILO8L')

    def test_hailo_identification_requires_the_architecture_field(self):
        for output in ('Board Name: Hailo-8\n', 'HAILO8L', ''):
            with self.subTest(output=output), self.assertRaisesRegex(RuntimeError, 'Missing Device Architecture'):
                check_hailo.device_kind(output)

    def test_hailo_identification_rejects_unsupported_architecture_without_board_fallback(self):
        for value in ('HAILO10H', 'HAILO8L_UNKNOWN', ''):
            output = f'Board Name: Hailo-8\nDevice Architecture: {value}\n'
            with self.subTest(value=value), self.assertRaisesRegex(RuntimeError, 'Unsupported Device Architecture'):
                check_hailo.device_kind(output)

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
