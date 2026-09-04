import json
import unittest

from orion_voice_worker.providers import Transcript
from orion_voice_worker.server import VoiceModels, transcribe_utterance
from orion_voice_worker.session import (
    PendingTranscription,
    SessionPhase,
    TranscriptionPurpose,
    VoiceSession,
)
from orion_voice_worker.tts import SpeechAudio


class FakeWebSocket:
    def __init__(self) -> None:
        self.messages: list[str | bytes] = []

    async def send(self, message: str | bytes) -> None:
        self.messages.append(message)


class FakeAsr:
    provider = "qwen3-asr"
    model_name = "test-asr"

    def transcribe(self, _pcm: bytes) -> Transcript:
        return Transcript("What time is it?", "English")


class FakeWake:
    provider = "rustpotter"
    model_name = "test-wake"
    threshold = 0.4

    def process(self, _pcm: bytes):
        return None

    def reset(self) -> None:
        pass


class FakeAgent:
    provider = "test-agent"
    model_name = "test-model"

    def respond(self, text: str) -> str:
        return f"I heard: {text}"

    def close(self) -> None:
        pass


class FakeTts:
    provider = "test-tts"
    model_name = "test-voice"

    def synthesize(self, text: str) -> SpeechAudio:
        self.text = text
        return SpeechAudio(b"\x01\x00\x02\x00", 24_000)


class ServerPipelineTests(unittest.IsolatedAsyncioTestCase):
    async def test_transcript_flows_through_agent_and_tts_to_binary_audio(self) -> None:
        asr = FakeAsr()
        session = VoiceSession(asr)
        session.phase = SessionPhase.TRANSCRIBING_COMMAND
        websocket = FakeWebSocket()
        models = VoiceModels(asr=asr, agent=FakeAgent(), tts=FakeTts())

        await transcribe_utterance(
            websocket,
            session,
            PendingTranscription(b"\x00\x00", TranscriptionPurpose.COMMAND),
            models,
            request_id=3,
        )

        controls = [json.loads(message) for message in websocket.messages if isinstance(message, str)]
        self.assertEqual(
            [message["type"] for message in controls],
            ["transcript.final", "agent.started", "agent.response", "synthesis.started", "speech.audio"],
        )
        self.assertEqual(controls[2]["text"], "I heard: What time is it?")
        self.assertEqual(websocket.messages[-1], b"\x01\x00\x02\x00")
        self.assertEqual(session.phase, SessionPhase.PLAYING_RESPONSE)


if __name__ == "__main__":
    unittest.main()
