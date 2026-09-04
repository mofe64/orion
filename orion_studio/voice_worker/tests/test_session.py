import unittest
from orion_voice_worker.providers import Transcript
from orion_voice_worker.session import VoiceSession, SessionPhase, command_after_wake_phrase

SID = 'a' * 32

class FakeAsr:
    pass

class VoiceSessionTests(unittest.TestCase):
    def pending(self):
        session = VoiceSession(FakeAsr())
        session.begin(SID)
        pending = session.accept_utterance(SID, 'wake_and_command', b'\x01\x00')
        return session, pending

    def test_confirmed_command_and_matching_playback(self):
        session, pending = self.pending()
        events = session.complete_transcription(pending.purpose, Transcript('Hey Orion, hello', 'English'))
        self.assertEqual(events[-1].fields['text'], 'hello')
        session.begin_playback(7)
        with self.assertRaises(ValueError): session.finish_playback(6)
        session.finish_playback(7)
        self.assertEqual(session.phase, SessionPhase.LISTENING)

    def test_false_positive_never_returns_command(self):
        session, pending = self.pending()
        events = session.complete_transcription(pending.purpose, Transcript('hello there', 'English'))
        self.assertEqual([e.type for e in events], ['wake.rejected'])
        self.assertIsNone(session.session_id)

    def test_bare_wake_requests_followup(self):
        session, pending = self.pending()
        events = session.complete_transcription(pending.purpose, Transcript('Hey, Orion!', 'English'))
        self.assertEqual(events[-1].type, 'command.started')
        pending = session.accept_utterance(SID, 'command', b'\x01\x00')
        final = session.complete_transcription(pending.purpose, Transcript('hello', 'English'))
        self.assertEqual(final[0].fields['text'], 'hello')

    def test_empty_followup_cannot_invoke_agent(self):
        session, pending = self.pending()
        session.complete_transcription(pending.purpose, Transcript('Hey Orion', 'English'))
        pending = session.accept_utterance(SID, 'command', b'\x00\x00')
        with self.assertRaises(ValueError):
            session.complete_transcription(pending.purpose, Transcript('', 'English'))

    def test_rejects_stale_overlapping_and_oversized_audio(self):
        session = VoiceSession(FakeAsr())
        session.begin(SID)
        with self.assertRaises(ValueError): session.begin(SID)
        for sid, data in [('b'*32, b'\x00\x00'), (SID,b'\x00'), (SID,b'\x00'*600000)]:
            with self.assertRaises(ValueError): session.accept_utterance(sid, 'wake_and_command', data)

    def test_phrase_must_begin_the_transcript(self):
        self.assertIsNone(command_after_wake_phrase('I said hey Orion'))
        self.assertEqual(command_after_wake_phrase('Hey, Orion: look up'), 'look up')
