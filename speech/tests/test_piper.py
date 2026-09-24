import io
import json
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import numpy as np

from orion_speech_worker.piper import PiperAlbaSynthesizer, is_piper_model
from orion_speech_worker.worker import serve


class FakeConfig:
    def __init__(self, **values):
        self.__dict__.update(values)

    def validate(self):
        return True


class FakeTts:
    sample_rate = 22_050
    samples = .25 * np.sin(2 * np.pi * 440 * np.arange(5 * 22_050) / 22_050)

    def __init__(self, config):
        self.config = config

    def generate(self, text, sid, speed):
        assert (text, sid, speed) == ('Orion is ready.', 0, 1.0)
        return SimpleNamespace(samples=self.samples, sample_rate=self.sample_rate)


class PiperTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / 'piper-alba-medium'
        (self.root / 'espeak-ng-data').mkdir(parents=True)
        (self.root / 'en_GB-alba-medium.onnx').write_bytes(b'fixture')
        (self.root / 'tokens.txt').write_text('fixture')
        self.module = SimpleNamespace(
            OfflineTtsConfig=FakeConfig,
            OfflineTtsModelConfig=FakeConfig,
            OfflineTtsVitsModelConfig=FakeConfig,
            OfflineTts=FakeTts,
        )
        environment = patch.dict(os.environ, {
            'ORION_SPEECH_BACKEND': 'pi',
            'ORION_PIPER_MODEL_DIR': str(self.root),
            'ORION_TTS_THREADS': '3',
        })
        previous = sys.modules.get('sherpa_onnx')
        sys.modules['sherpa_onnx'] = self.module
        environment.start()
        self.addCleanup(environment.stop)
        def restore_module():
            if previous is None:
                sys.modules.pop('sherpa_onnx', None)
            else:
                sys.modules['sherpa_onnx'] = previous
        self.addCleanup(restore_module)

    def test_pinned_model_resamples_to_bounded_24khz_chunks(self):
        self.assertTrue(is_piper_model('piper-alba-medium'))
        self.assertTrue(is_piper_model(str(self.root)))
        tts = PiperAlbaSynthesizer('piper-alba-medium')
        self.assertEqual(tts.model.config.model.num_threads, 3)
        self.assertEqual(tts.model.config.model.vits.model, str(self.root / 'en_GB-alba-medium.onnx'))
        chunks = list(tts.stream('Orion is ready.'))
        self.assertEqual([chunk.samples for chunk in chunks], [48_000, 48_000, 24_000])
        self.assertTrue(all(chunk.sample_rate == 24_000 for chunk in chunks))
        pcm = np.frombuffer(b''.join(chunk.pcm for chunk in chunks), dtype='<i2')
        self.assertEqual(len(pcm), 5 * 24_000)
        self.assertEqual(round(np.fft.rfftfreq(len(pcm), 1 / 24_000)[np.argmax(abs(np.fft.rfft(pcm)))]), 440)

    def test_worker_reports_piper_and_preserves_wire_chunk_limits(self):
        config = {'protocol': 2, 'role': 'tts', 'asr_model': 'unused', 'tts_model': 'piper-alba-medium'}
        job = {'method': 'synthesize', 'id': 1, 'text': 'Orion is ready.', 'voice': 'jane'}
        reader = io.BytesIO((json.dumps(config) + '\n' + json.dumps(job) + '\n').encode())
        writer = io.BytesIO()
        serve(reader, writer)
        output = io.BytesIO(writer.getvalue())
        ready = json.loads(output.readline())
        self.assertEqual(ready['tts'], {'provider': 'piper-tts', 'model': 'piper-alba-medium'})
        sizes = []
        while True:
            message = json.loads(output.readline())
            if message['type'] == 'end':
                self.assertEqual(message['sequence'], len(sizes))
                break
            self.assertEqual(message['sampleRate'], 24_000)
            self.assertLessEqual(message['samples'], 48_000)
            sizes.append(message['samples'])
            self.assertEqual(len(output.read(message['samples'] * 2)), message['samples'] * 2)
        self.assertEqual(sizes, [48_000, 48_000, 24_000])

    def test_bad_audio_and_missing_weights_fail_before_pcm(self):
        tts = PiperAlbaSynthesizer('piper-alba-medium')
        tts.model.samples = np.array([np.nan], dtype=np.float32)
        with self.assertRaisesRegex(ValueError, 'invalid audio'):
            list(tts.stream('Orion is ready.'))
        (self.root / 'tokens.txt').unlink()
        with self.assertRaisesRegex(ValueError, 'incomplete'):
            PiperAlbaSynthesizer('piper-alba-medium')


if __name__ == '__main__':
    unittest.main()
