import json
import unittest

from orion_voice_worker.protocol import ProtocolError, event, parse_hello


def hello(**overrides: object) -> str:
    message = {
        "type": "hello",
        "protocol": 7,
        "token": "secret",
    }
    message.update(overrides)
    return json.dumps(message)


class ProtocolTests(unittest.TestCase):
    def test_accepts_control_handshake(self) -> None:
        parsed = parse_hello(hello())
        self.assertEqual(parsed.token, "secret")

    def test_rejects_previous_protocol(self) -> None:
        with self.assertRaisesRegex(ProtocolError, "protocol"):
            parse_hello(hello(protocol=5))

    def test_serializes_compact_events(self) -> None:
        self.assertEqual(event("command.started"), '{"type":"command.started"}')


if __name__ == "__main__":
    unittest.main()
