import unittest
from unittest.mock import Mock

import numpy as np

from orion_voice.satellite import SatelliteSession
from test_satellite import Wake, frame
from test_verifier import verifier


class BargeInTests(unittest.TestCase):
    def playing(self, scores=()):
        self.wake = Wake()
        self.verifier = verifier(scores)
        self.direction = Mock()
        session = SatelliteSession(self.wake, verifier=self.verifier,
            direction=self.direction, barge_in=True)
        session.session_id = 'a' * 32
        session.phase = 'processing'
        session.control({'type': 'session.playing', 'sessionId': session.session_id})
        return session

    def test_accepted_playback_wake_interrupts_then_starts_verified_session_from_pre_roll(self):
        session = self.playing({1: .9})
        old = session.session_id
        for _ in range(4): self.assertEqual(session.accept_stereo(frame(1234)), [])
        self.wake.next = True
        events = session.accept_stereo(frame(2345))
        self.assertEqual([e['type'] for e in events],
            ['session.interrupted', 'wake.candidate', 'wake.verified'])
        self.assertEqual(events[0], {'type': 'session.interrupted', 'sessionId': old, 'reason': 'barge_in'})
        self.assertNotEqual(session.session_id, old)
        self.assertEqual(events[1]['sessionId'], events[2]['sessionId'])
        self.assertTrue(events[1]['acousticVerification'])
        self.assertTrue(events[2]['accepted'])
        self.assertTrue(session.barge_in_session)
        self.assertFalse(session.acoustic_pending)
        self.assertEqual(session.phase, 'wake')
        self.assertEqual(events[1]['direction']['side'], 'unknown')
        self.assertEqual(np.frombuffer(session.utterance, dtype='<i2')[0], 1234)
        for _ in range(15): session.accept_stereo(frame(3456))
        for _ in range(100):
            events = session.accept_stereo(frame(0))
            if events: break
        self.assertEqual(events[0]['purpose'], 'wake_and_command')
        samples = np.frombuffer(events[1], dtype='<i2')
        for value in (1234, 2345, 3456): self.assertIn(value, samples)
        self.direction.accept.assert_not_called()
        self.direction.observation.assert_not_called()
        with self.assertRaises(ValueError):
            session.control({'type': 'session.finish', 'sessionId': old})

    def test_verification_waits_while_playback_continues_and_rejection_emits_nothing(self):
        session = self.playing()
        old = session.session_id
        self.wake.next = True
        self.assertEqual(session.accept_stereo(frame()), [])
        self.assertIsNotNone(session.barge_candidate)
        for _ in range(60):
            self.assertEqual(session.accept_stereo(frame(3456)), [])
            self.assertEqual(session.phase, 'playing')
            self.assertEqual(session.session_id, old)
        self.assertIsNone(session.barge_candidate)
        self.assertLessEqual(len(session.pre_roll), 3 * 32000)

    def test_later_acceptance_retains_audio_during_verification(self):
        session = self.playing({3: .8})
        self.wake.next = True
        self.assertEqual(session.accept_stereo(frame(1234)), [])
        for _ in range(10): self.assertEqual(session.accept_stereo(frame(3456)), [])
        events = session.accept_stereo(frame(4567))
        self.assertEqual(events[0]['type'], 'session.interrupted')
        self.assertIn(1234, np.frombuffer(session.utterance, dtype='<i2'))
        self.assertIn(3456, np.frombuffer(session.utterance, dtype='<i2'))
        self.assertIn(4567, np.frombuffer(session.utterance, dtype='<i2'))

    def test_absent_unhealthy_or_disabled_verifier_never_runs_playback_wake(self):
        for mode in ('absent', 'unhealthy', 'disabled'):
            session = self.playing({1: .9})
            if mode == 'absent': session.verifier = None
            elif mode == 'unhealthy': self.verifier.healthy = False
            else: session.barge_in = False
            self.wake.process = Mock(side_effect=AssertionError('Wake must remain off'))
            self.assertEqual(session.accept_stereo(frame()), [])
            self.wake.process.assert_not_called()

    def test_processing_does_not_run_wake_and_cancels_pending_playback_candidate(self):
        session = self.playing()
        self.wake.next = True
        session.accept_stereo(frame())
        session.control({'type': 'session.processing', 'sessionId': session.session_id})
        self.assertIsNone(session.barge_candidate)
        self.wake.process = Mock(side_effect=AssertionError('No processing wake'))
        self.assertEqual(session.accept_stereo(frame()), [])

    def test_alarm_preempts_pending_barge_in_and_consumes_wake(self):
        session = self.playing({3: .9})
        old = session.session_id
        self.wake.next = True
        self.assertEqual(session.accept_stereo(frame()), [])
        self.assertEqual(session.set_alarm(True), old)
        self.assertIsNone(session.barge_candidate)
        self.wake.next = True
        self.assertEqual(session.accept_stereo(frame()), [{'type': 'alarm.dismiss'}])
        self.assertIsNone(session.session_id)

    def test_verifier_becoming_unhealthy_abandons_pending_candidate(self):
        session = self.playing({3: .9})
        self.wake.next = True
        session.accept_stereo(frame())
        self.verifier.healthy = False
        self.assertEqual(session.accept_stereo(frame()), [])
        self.assertIsNone(session.barge_candidate)
        self.assertEqual(session.phase, 'playing')

    def test_playback_completion_discards_pending_verification_before_followup_guard(self):
        session = self.playing({3: .9})
        old = session.session_id
        self.wake.next = True
        session.accept_stereo(frame())
        session.control({'type': 'session.finish', 'sessionId': old, 'conversationWindow': True})
        self.assertIsNone(session.barge_candidate)
        self.assertIsNone(self.verifier.candidate)
        for _ in range(11):
            self.assertEqual(session.accept_stereo(frame()), [])
        self.assertEqual(session.phase, 'echo_guard')
        self.assertEqual(session.session_id, old)
