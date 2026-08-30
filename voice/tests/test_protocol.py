import json
import unittest

from orion_voice.protocol import MAX_SPEECH_TEXT_BYTES, SynthesisRequest


class SynthesisRequestTests(unittest.TestCase):
    def test_round_trips_valid_request(self) -> None:
        request = SynthesisRequest.from_json_line(
            SynthesisRequest(7, "Hello, Orion.").to_json_line().rstrip(b"\n")
        )
        self.assertEqual(request, SynthesisRequest(7, "Hello, Orion."))

    def test_rejects_invalid_ids_and_text(self) -> None:
        for value in [
            {"request_id": 0, "text": "hello"},
            {"request_id": True, "text": "hello"},
            {"request_id": 1, "text": "   "},
            {"request_id": 1, "text": "hello\nthere"},
            {"request_id": 1, "text": "x" * (MAX_SPEECH_TEXT_BYTES + 1)},
        ]:
            with self.subTest(value=value):
                with self.assertRaises(ValueError):
                    SynthesisRequest.from_json_line(json.dumps(value).encode())


if __name__ == "__main__":
    unittest.main()
