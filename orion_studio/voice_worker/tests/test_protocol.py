import json
import unittest

from orion_voice_worker.protocol import ProtocolError, event, parse_hello


def hello(**overrides: object) -> str:
    message = {
        "type": "hello",
        "protocol": 5,
        "token": "secret",
        "sampleRate": 16_000,
        "channels": 1,
        "encoding": "pcm_s16le",
        "frameSamples": 1_280,
    }
    message.update(overrides)
    return json.dumps(message)


class ProtocolTests(unittest.TestCase):
    def test_accepts_exact_audio_contract(self) -> None:
        parsed = parse_hello(hello())
        self.assertEqual(parsed.token, "secret")

    def test_rejects_wrong_sample_rate(self) -> None:
        with self.assertRaisesRegex(ProtocolError, "sampleRate"):
            parse_hello(hello(sampleRate=48_000))

    def test_serializes_compact_events(self) -> None:
        self.assertEqual(event("command.started"), '{"type":"command.started"}')


if __name__ == "__main__":
    unittest.main()
