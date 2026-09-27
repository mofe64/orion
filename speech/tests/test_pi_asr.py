import base64
import io
import os
import unittest
import wave
from unittest.mock import patch

from orion_speech_worker.pi import QwenGgufTranscriber


class PiAsrTests(unittest.TestCase):
    def transcribe(self, output):
        transcriber = object.__new__(QwenGgufTranscriber)
        requests = []

        def request(path, payload):
            requests.append((path, payload))
            return {'choices': [{'finish_reason': 'stop', 'message': {'content': output}}]}

        transcriber.request = request
        with patch.dict(os.environ, {'ORION_ASR_CONTEXT': 'Orion'}, clear=True):
            transcript = transcriber.transcribe(b'\x00\x00' * 16000)
        return transcript, requests[0]

    def test_forces_english_in_qwen_request_and_accepts_prefilled_response(self):
        transcript, (path, payload) = self.transcribe(
            'language English<asr_text>Turn on some ambient lighting.'
        )
        self.assertEqual((transcript.text, transcript.language),
                         ('Turn on some ambient lighting.', 'English'))
        self.assertEqual(path, '/v1/chat/completions')
        self.assertEqual(payload['messages'][-1],
                         {'role': 'assistant', 'content': 'language English<asr_text>'})
        encoded = payload['messages'][1]['content'][0]['input_audio']['data']
        with wave.open(io.BytesIO(base64.b64decode(encoded))) as audio:
            self.assertEqual((audio.getnchannels(), audio.getframerate(), audio.getnframes()),
                             (1, 16000, 16000))

    def test_accepts_text_only_prefill_response(self):
        transcript, _ = self.transcribe('Turn on some ambient lighting.')
        self.assertEqual((transcript.text, transcript.language),
                         ('Turn on some ambient lighting.', 'English'))

    def test_rejects_language_metadata_that_conflicts_with_english_prefill(self):
        with self.assertRaisesRegex(ValueError, 'English transcription prefix'):
            self.transcribe('language Portuguese<asr_text>todo um caminho diferente')

    def test_rejects_language_metadata_without_transcript_marker(self):
        with self.assertRaisesRegex(ValueError, 'unparsed language metadata'):
            self.transcribe('language Portuguese todo um caminho diferente')


if __name__ == '__main__':
    unittest.main()
