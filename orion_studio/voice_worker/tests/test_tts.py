from types import SimpleNamespace
import unittest

import numpy as np

from orion_voice_worker.tts import ChatterboxSynthesizer


class FakeModel:
    def __init__(self, results: list[object]) -> None:
        self.results = results

    def generate(self, **_options: object):
        yield from self.results


class ChatterboxSynthesizerTests(unittest.TestCase):
    def test_converts_generated_float_audio_to_pcm16(self) -> None:
        model = FakeModel([
            SimpleNamespace(audio=np.array([-1.0, 0.0, 1.0]), sample_rate=24_000),
        ])
        synthesizer = ChatterboxSynthesizer("test", model_loader=lambda _name: model)
        audio = synthesizer.synthesize("Hello")

        self.assertEqual(audio.sample_rate, 24_000)
        self.assertEqual(audio.samples, 3)
        self.assertEqual(np.frombuffer(audio.pcm, dtype="<i2").tolist(), [-32767, 0, 32767])

    def test_rejects_empty_or_inconsistent_generation(self) -> None:
        empty = ChatterboxSynthesizer("test", model_loader=lambda _name: FakeModel([]))
        with self.assertRaisesRegex(RuntimeError, "no audio"):
            empty.synthesize("Hello")

        mixed = FakeModel([
            SimpleNamespace(audio=np.array([0.0]), sample_rate=24_000),
            SimpleNamespace(audio=np.array([0.0]), sample_rate=16_000),
        ])
        synthesizer = ChatterboxSynthesizer("test", model_loader=lambda _name: mixed)
        with self.assertRaisesRegex(RuntimeError, "changed sample rate"):
            synthesizer.synthesize("Hello")

        invalid_rate = FakeModel([
            SimpleNamespace(audio=np.array([0.0]), sample_rate=0),
        ])
        synthesizer = ChatterboxSynthesizer("test", model_loader=lambda _name: invalid_rate)
        with self.assertRaisesRegex(RuntimeError, "invalid sample rate"):
            synthesizer.synthesize("Hello")


if __name__ == "__main__":
    unittest.main()
