import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('prepare', Path(__file__).parents[1] / 'prepare_pi_voice_stack.py')
prepare = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(prepare)


class PreparationTests(unittest.TestCase):
    def test_fresh_download_is_verified_recorded_and_reused_offline(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            payload = b'pinned model fixture'
            digest = hashlib.sha256(payload).hexdigest()
            with patch.object(prepare.urllib.request, 'urlopen', return_value=io.BytesIO(payload)) as fetch:
                path = prepare.download(root, 'https://example.test/model', 'models/qwen/model.gguf', digest)
                prepare.download(root, 'https://example.test/model', 'models/qwen/model.gguf', digest)
                fetch.assert_called_once()
            self.assertEqual(path.read_bytes(), payload)
            entries = json.loads((root / 'downloads.json').read_text())
            self.assertEqual(len(entries), 1)
            self.assertEqual(entries[0]['sha256'], digest)

    def test_bad_download_never_publishes_partial_model(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.object(prepare.urllib.request, 'urlopen', return_value=io.BytesIO(b'bad')):
                with self.assertRaisesRegex(RuntimeError, 'Checksum mismatch'):
                    prepare.download(root, 'https://example.test/model', 'model.gguf', '0' * 64)
            self.assertFalse((root / 'model.gguf').exists())
            self.assertFalse((root / 'model.gguf.download').exists())

    def test_existing_changed_model_is_preserved_and_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / 'model'; path.write_bytes(b'custom model')
            with self.assertRaisesRegex(RuntimeError, 'left unchanged'):
                prepare.download(root, 'https://example.test/model', 'model', '0' * 64)
            self.assertEqual(path.read_bytes(), b'custom model')

    def test_fresh_archive_keeps_server_libraries_without_experiment_tree(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            stream = io.BytesIO()
            with tarfile.open(fileobj=stream, mode='w:gz') as archive:
                for name in ['llama-fixture/llama-server', 'llama-fixture/libggml.so']:
                    info = tarfile.TarInfo(name); info.size = 7; info.mode = 0o755
                    archive.addfile(info, io.BytesIO(b'fixture'))
            payload = stream.getvalue()
            with patch.object(prepare.urllib.request, 'urlopen', return_value=io.BytesIO(payload)):
                prepare.prepare_archive(root, 'llama-fixture', 'https://example.test/llama',
                    hashlib.sha256(payload).hexdigest(), 'llama-server', nested=True)
            self.assertEqual((root / 'llama-fixture/libggml.so').read_bytes(), b'fixture')
            self.assertEqual(list(root.glob('.extract-*')), [])

    def test_piper_archive_is_verified_and_corrupt_reuse_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            stream = io.BytesIO()
            members = {
                'vits-piper-en_GB-alba-medium/en_GB-alba-medium.onnx': b'weights',
                'vits-piper-en_GB-alba-medium/tokens.txt': b'tokens',
                'vits-piper-en_GB-alba-medium/espeak-ng-data/example': b'data',
            }
            with tarfile.open(fileobj=stream, mode='w:bz2') as archive:
                for name, content in members.items():
                    info = tarfile.TarInfo(name); info.size = len(content)
                    archive.addfile(info, io.BytesIO(content))
            payload = stream.getvalue()
            with patch.object(prepare, 'PIPER_ALBA_SHA256', hashlib.sha256(payload).hexdigest()), \
                 patch.object(prepare, 'PIPER_ALBA_WEIGHTS_SHA256', hashlib.sha256(b'weights').hexdigest()), \
                 patch.object(prepare, 'PIPER_ALBA_TOKENS_SHA256', hashlib.sha256(b'tokens').hexdigest()), \
                 patch.object(prepare.urllib.request, 'urlopen', return_value=io.BytesIO(payload)) as fetch:
                prepare.prepare_piper_alba(root)
                prepare.prepare_piper_alba(root)
                fetch.assert_called_once()
                target = root / 'models/piper-alba-medium'
                self.assertEqual((target / 'en_GB-alba-medium.onnx').read_bytes(), b'weights')
                self.assertEqual(list(root.glob('.extract-piper-*')), [])
                (target / 'en_GB-alba-medium.onnx').write_bytes(b'changed')
                with self.assertRaisesRegex(RuntimeError, 'checksum mismatch'):
                    prepare.prepare_piper_alba(root)


if __name__ == '__main__':
    unittest.main()
