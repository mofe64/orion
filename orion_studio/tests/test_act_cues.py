import array
import unittest
import wave
from pathlib import Path


class ActCueTests(unittest.TestCase):
    def test_rare_cues_are_short_and_quieter_than_voice_wake(self):
        directory = Path(__file__).resolve().parents[2] / 'audio/cues'
        def read(name):
            with wave.open(str(directory / f'{name}.wav'), 'rb') as wav:
                self.assertEqual((wav.getnchannels(), wav.getsampwidth(), wav.getframerate()), (2, 2, 48000))
                seconds = wav.getnframes() / wav.getframerate()
                samples = array.array('h', wav.readframes(wav.getnframes()))
                self.assertEqual(samples[::2], samples[1::2])
                return seconds, max(abs(value) for value in samples)
        _, voice_peak = read('voice_wake')
        for name in ('wake_chirp', 'sleep_sigh', 'yawn_hum'):
            with self.subTest(cue=name):
                seconds, peak = read(name)
                self.assertGreater(seconds, 0)
                self.assertLess(seconds, 1.2)
                self.assertGreater(peak, 0)
                self.assertLess(peak, voice_peak)
