from array import array
import math
from pathlib import Path
import sys
import unittest
import wave


class VoiceCueTests(unittest.TestCase):
    def test_cues_are_stereo_with_matching_channels_bounded_lengths_and_peaks(self):
        directory = Path(__file__).resolve().parents[2] / 'audio/cues'
        cues = sorted(directory.glob('*.wav'))
        self.assertGreaterEqual(len(cues), 20)
        for path in cues:
            with wave.open(str(path)) as audio:
                self.assertEqual((audio.getframerate(), audio.getnchannels(), audio.getsampwidth()), (48000, 2, 2), path.name)
                seconds = audio.getnframes() / 48000
                samples = array('h', audio.readframes(audio.getnframes()))
            if sys.byteorder != 'little': samples.byteswap()
            mono = samples[::2]
            self.assertEqual(mono, samples[1::2], path.name)
            peak = 20 * math.log10(max(abs(x) for x in mono) / 32767)
            self.assertTrue(0.2 <= seconds <= 3.0, path.name)
            self.assertTrue(-21 <= peak <= -11, path.name)
            # A cue starts and ends in silence, so playback does not click.
            self.assertLess(max(abs(x) for x in mono[:48]), 400, path.name)
            self.assertLess(max(abs(x) for x in mono[-48:]), 400, path.name)

    def test_voice_cues_keep_their_levels_and_the_wake_cue_stays_short(self):
        directory = Path(__file__).resolve().parents[2] / 'audio/cues'
        for name, dbfs, longest in (('voice_wake', -12, 0.45), ('voice_processing', -18, 0.7)):
            with wave.open(str(directory / (name + '.wav'))) as audio:
                seconds = audio.getnframes() / 48000
                samples = array('h', audio.readframes(audio.getnframes()))
            if sys.byteorder != 'little': samples.byteswap()
            self.assertAlmostEqual(20 * math.log10(max(abs(x) for x in samples) / 32767), dbfs, delta=0.1)
            self.assertLessEqual(seconds, longest)


if __name__ == '__main__':
    unittest.main()
