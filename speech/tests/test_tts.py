import unittest

from orion_speech_worker.tts import SpeechAudio, speech_segments


class SpeechSegmentsTests(unittest.TestCase):
    def test_long_sentences_are_bounded_without_losing_words(self):
        text = ' '.join(['something'] * 80) + '.'
        segments = list(speech_segments(text))
        self.assertTrue(all(len(segment) <= 160 for segment in segments))
        self.assertEqual(' '.join(segments), text)
        self.assertEqual(''.join(speech_segments('x' * 500)), 'x' * 500)

    def test_splits_sentences_and_lines_without_empty_segments(self):
        self.assertEqual(list(speech_segments(' Hello there. How are you?\n\nFine! ')),
                         ['Hello there.', 'How are you?', 'Fine!'])

    def test_pcm_audio_reports_sample_count_and_duration(self):
        audio = SpeechAudio(pcm=b'\0\0' * 24_000, sample_rate=24_000)
        self.assertEqual(audio.samples, 24_000)
        self.assertEqual(audio.duration_ms, 1000)


if __name__ == '__main__':
    unittest.main()
