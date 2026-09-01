from array import array
import unittest

from orion_voice_worker.providers import Transcript
from orion_voice_worker.session import (
    SessionPhase,
    TranscriptionPurpose,
    VoiceSession,
    command_after_wake_phrase,
)
from orion_voice_worker.wake import WakeDetection


def pcm(value: int, samples: int = 1_280) -> bytes:
    return array("h", [value] * samples).tobytes()


class FakeAsr:
    provider = "qwen3-asr"
    model_name = "Qwen/Qwen3-ASR-0.6B"

    def transcribe(self, _pcm: bytes) -> Transcript:
        return Transcript("Hey Orion, turn left", "English")


class FakeWake:
    provider = "rustpotter"
    model_name = "hey_orion_reference.rpw"
    threshold = 0.4

    def __init__(self) -> None:
        self.detect_next = False
        self.reset_count = 0

    def process(self, _pcm: bytes) -> WakeDetection | None:
        if not self.detect_next:
            return None
        self.detect_next = False
        return WakeDetection("hey_orion", 0.62)

    def reset(self) -> None:
        self.reset_count += 1


class VoiceSessionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.wake = FakeWake()
        self.session = VoiceSession(FakeAsr(), self.wake)

    def detect_and_endpoint(self):
        self.session.accept_audio(pcm(0))
        self.wake.detect_next = True
        events, pending = self.session.accept_audio(pcm(2_000))
        self.assertEqual(events[0].type, "wake.candidate")
        self.assertIsNone(pending)

        for _ in range(3):
            _, pending = self.session.accept_audio(pcm(2_000))
        for _ in range(12):
            _, pending = self.session.accept_audio(pcm(0))
            if pending is not None:
                break
        self.assertIsNotNone(pending)
        return pending

    def test_confirms_wake_and_returns_command_from_one_asr_pass(self) -> None:
        pending = self.detect_and_endpoint()
        self.assertEqual(pending.purpose, TranscriptionPurpose.WAKE)

        events = self.session.complete_transcription(
            pending.purpose,
            Transcript("Hey Orion, turn towards me.", "English"),
        )
        self.assertEqual([event.type for event in events], ["wake.confirmed", "transcript.final"])
        self.assertEqual(events[1].fields["text"], "turn towards me.")
        self.assertEqual(self.session.phase, SessionPhase.PROCESSING_RESPONSE)
        self.assertEqual(self.wake.reset_count, 0)

        self.session.begin_playback(1)
        self.assertEqual(self.session.phase, SessionPhase.PLAYING_RESPONSE)
        self.assertEqual(self.session.accept_audio(pcm(2_000)), ([], None))
        self.session.finish_playback(1)
        self.assertEqual(self.session.phase, SessionPhase.LISTENING)
        self.assertEqual(self.wake.reset_count, 1)

    def test_rejects_candidate_when_asr_does_not_hear_wake_phrase(self) -> None:
        pending = self.detect_and_endpoint()
        events = self.session.complete_transcription(
            pending.purpose,
            Transcript("we should turn left", "English"),
        )
        self.assertEqual([event.type for event in events], ["wake.rejected"])
        self.assertEqual(self.session.phase, SessionPhase.LISTENING)

    def test_bare_wake_phrase_opens_a_followup_command(self) -> None:
        pending = self.detect_and_endpoint()
        events = self.session.complete_transcription(
            pending.purpose,
            Transcript("Hey, Orion!", "English"),
        )
        self.assertEqual([event.type for event in events], ["wake.confirmed", "command.started"])
        self.assertEqual(self.session.phase, SessionPhase.CAPTURING_COMMAND)

        for _ in range(3):
            _, followup = self.session.accept_audio(pcm(2_000))
        for _ in range(12):
            _, followup = self.session.accept_audio(pcm(0))
            if followup is not None:
                break
        self.assertEqual(followup.purpose, TranscriptionPurpose.COMMAND)
        final = self.session.complete_transcription(
            followup.purpose,
            Transcript("turn left", "English"),
        )
        self.assertEqual(final[0].fields["text"], "turn left")

    def test_rejects_partial_pcm_without_losing_listening_state(self) -> None:
        events, pending = self.session.accept_audio(b"\x01")
        self.assertIsNone(pending)
        self.assertEqual(events[0].fields["code"], "invalid_pcm")
        self.assertEqual(self.session.phase, SessionPhase.LISTENING)


class WakePhraseParsingTests(unittest.TestCase):
    def test_extracts_command_after_punctuation(self) -> None:
        self.assertEqual(command_after_wake_phrase("Hey, Orion: look up"), "look up")

    def test_returns_empty_string_for_wake_phrase_only(self) -> None:
        self.assertEqual(command_after_wake_phrase("hey orion!"), "")

    def test_rejects_phrase_that_is_not_at_the_start(self) -> None:
        self.assertIsNone(command_after_wake_phrase("I said hey Orion"))


if __name__ == "__main__":
    unittest.main()
