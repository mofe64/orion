"""Conditional, checksum-protected HEF installation without a Hailo device."""
import hashlib
import io
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
import vision_assets


class HailoAssetTests(unittest.TestCase):
    def test_without_device_does_not_probe_or_download(self):
        with patch.object(vision_assets, 'hailo_models') as models:
            vision_assets.prepare_hailo_models(Path('/unused'), False, Mock())
        models.assert_not_called()

    def test_palm_pin_is_orion_hosted_and_only_supplied_for_hailo8l(self):
        models = vision_assets.hailo_models()
        for architecture, entries in models.model_manifest().items():
            self.assertEqual(set(entries), {'faces', 'objects', 'palms'} if architecture == 'HAILO8L' else {'faces', 'objects'})
            for name, entry in entries.items():
                if name == 'palms':
                    self.assertTrue(entry['url'].startswith('https://github.com/mofe64/orion/releases/download/'))
                    self.assertEqual(entry['sha256'], '63d3be909506d253b05b0a8aa048451358ca66e617a2335c316907675d64df7d')
                else:
                    self.assertIn('/v2.14.0/' + architecture.lower() + '/', entry['url'])
                self.assertTrue(entry['path'].startswith(architecture.lower() + '/'))
                self.assertEqual(len(bytes.fromhex(entry['sha256'])), 32)

    def test_8l_downloads_hash_checks_and_reuses_global_files(self):
        models = vision_assets.hailo_models()
        body = b'fixed HEF fixture'
        pins = {'HAILO8L': {name: {'url': 'https://example.invalid/' + name,
                        'path': 'hailo8l/' + name + '.hef', 'sha256': hashlib.sha256(body).hexdigest()}
                        for name in ('faces', 'objects', 'palms')}}
        def install(*args):
            if args[1:3] == ('install', '-d'):
                Path(args[-1]).mkdir(parents=True, exist_ok=True)
            else:
                shutil.copyfile(args[-2], args[-1])
        with tempfile.TemporaryDirectory() as directory, \
             patch.object(models, 'device_architecture', return_value='HAILO8L'), \
             patch.object(models, 'model_manifest', return_value=pins):
            root = Path(directory) / 'cache'; global_models = Path(directory) / 'global'
            with patch('prepare_pi_voice_stack.urllib.request.urlopen', side_effect=lambda *a, **kw: io.BytesIO(body)) as fetch:
                vision_assets.prepare_hailo_models(root, True, install, global_models)
            self.assertEqual(fetch.call_count, 3)
            self.assertEqual({path.name for path in global_models.rglob('*.hef')}, {'faces.hef', 'objects.hef', 'palms.hef'})
            with patch('prepare_pi_voice_stack.urllib.request.urlopen', side_effect=AssertionError('downloaded twice')):
                vision_assets.prepare_hailo_models(root, True, install, global_models)
            paths = models.checked_model_paths('HAILO8L', global_models)
            paths['faces'].write_bytes(b'corrupt')
            with self.assertRaisesRegex(RuntimeError, 'SHA-256 mismatch'), \
                 patch('prepare_pi_voice_stack.urllib.request.urlopen', side_effect=AssertionError('replaced corrupt file')):
                vision_assets.prepare_hailo_models(root, True, install, global_models)

    def test_hailo8_downloads_no_palm_file(self):
        models = vision_assets.hailo_models()
        with tempfile.TemporaryDirectory() as directory, \
             patch.object(models, 'device_architecture', return_value='HAILO8'), \
             patch('prepare_pi_voice_stack.download', return_value=Path(directory) / 'downloaded') as fetch:
            vision_assets.prepare_hailo_models(Path(directory), True, Mock(), Path(directory) / 'global')
        self.assertEqual(fetch.call_count, 2)
        self.assertTrue(all('/hailo8/' in call.args[1] for call in fetch.call_args_list))
        self.assertTrue(all('palm' not in call.args[1] for call in fetch.call_args_list))

    def test_bad_download_is_never_installed(self):
        models = vision_assets.hailo_models()
        with tempfile.TemporaryDirectory() as directory, \
             patch.object(models, 'device_architecture', return_value='HAILO8L'), \
             patch('prepare_pi_voice_stack.urllib.request.urlopen', return_value=io.BytesIO(b'wrong HEF')):
            run = Mock()
            with self.assertRaisesRegex(RuntimeError, 'Checksum mismatch'):
                vision_assets.prepare_hailo_models(Path(directory), True, run, Path(directory) / 'global')
            run.assert_not_called()
            self.assertEqual(list(Path(directory).rglob('*.hef')), [])

    def test_device_not_ready_can_be_retried_after_reboot(self):
        models = vision_assets.hailo_models()
        with patch.object(models, 'device_architecture', side_effect=FileNotFoundError('hailortcli')), \
             patch('prepare_pi_voice_stack.download') as fetch, patch('builtins.print') as message:
            vision_assets.prepare_hailo_models(Path('/unused'), True, Mock())
        fetch.assert_not_called()
        self.assertIn('vision will use CPU', message.call_args.args[0])
