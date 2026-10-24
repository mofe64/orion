from array import array
import math
from pathlib import Path
import sys
import unittest
import wave


class VoiceCueTests(unittest.TestCase):
    def test_voice_cues_have_audible_notes_exact_lengths_and_bounded_peaks(self):
        directory = Path(__file__).resolve().parents[2] / 'audio/cues'
        for name, dbfs, duration, notes in (
            ('voice_wake', -12, .225, [(0, .09, 880), (.105, .12, 1320)]),
            ('voice_processing', -18, .12, [(0, .12, 660)]),
        ):
            with wave.open(str(directory / (name + '.wav'))) as audio:
                self.assertEqual((audio.getframerate(), audio.getnchannels()), (48000, 2))
                self.assertEqual(audio.getnframes(), round(duration * 48000))
                samples = array('h', audio.readframes(audio.getnframes()))
            if sys.byteorder != 'little': samples.byteswap()
            mono = samples[::2]
            self.assertEqual(mono, samples[1::2])
            self.assertAlmostEqual(20 * math.log10(max(abs(x) for x in mono) / 32767), dbfs, places=2)
            for start, length, hz in notes:
                segment = mono[round((start+.01)*48000):round((start+length-.01)*48000)]
                crossings = sum(a <= 0 < b for a,b in zip(segment, segment[1:]))
                estimated = crossings / ((length-.02))
                self.assertAlmostEqual(estimated, hz, delta=20)
