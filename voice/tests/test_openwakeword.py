from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import Mock

from orion_voice.satellite import SatelliteSession, WAKE_PREFIX_TAIL_MS
from orion_voice.verifier import OpenWakeWordDetector
from test_satellite import frame
from test_verifier import ScriptedScorer, MODELS


class ContinuousWakeTests(unittest.TestCase):
    def test_warmup_cooldown_and_capture_gap(self):
        scorer = ScriptedScorer({1: .9, 2: .5, 3: .8, 26: .9, 27: .7}, warmup=1)
        detector = OpenWakeWordDetector(MODELS, scorer=scorer)
        hits = [i for i in range(1, 109) if detector.process(b'\0' * 640)]
        self.assertEqual(hits, [8, 108])
        detector.reset()
        self.assertEqual(scorer.samples, 0)

    def detector(self):
        detector = Mock(provider='openwakeword', model_name='test', threshold=.5)
        detector.process.return_value = None
        return detector

    def test_every_phase_scores_once_and_turn_changes_keep_the_stream(self):
        wake = self.detector()
        session = SatelliteSession(wake)
        for phase in ('listening', 'wake', 'command', 'confirming', 'processing', 'playing', 'echo_guard', 'conversation'):
            session.reset()
            session.phase = phase
            session.session_id = 'a' * 32
            session.accept_stereo(frame(0))
        session.alarm_active = True
        session.accept_stereo(frame(0))
        session.alarm_active = False
        session.reset(preserve_listening=True)
        session.start_announcement({'id': 1, 'attempt': 0, 'text': 'test', 'purpose': 'announcement'})
        session.accept_stereo(frame(0))
        self.assertEqual(wake.process.call_count, 10)
        self.assertEqual(len(session.pre_roll), 10 * 640)
        wake.reset.assert_not_called()
        session.restart_verifier()
        wake.reset.assert_called_once()

    def test_prefix_is_full_preroll_plus_exact_tail_and_rejection_keeps_state(self):
        wake = self.detector()
        session = SatelliteSession(wake, early_wake=True)
        history = []
        for i in range(150):
            history.append(frame(i))
            session.accept_stereo(history[-1])
        wake.process.return_value = SimpleNamespace(name='Hey Orion', score=.8)
        event = session.accept_stereo(frame(900))
        identity = event[0]['sessionId']
        expected = b''.join(history[1:] + [frame(900)])
        wake.process.return_value = None
        for _ in range(WAKE_PREFIX_TAIL_MS // 20):
            result = session.accept_stereo(frame(321))
        mono = lambda stereo: b''.join(stereo[i:i+2] for i in range(0, len(stereo), 4))
        self.assertEqual(result[1], mono(expected) + mono(frame(321)) * 15)
        self.assertEqual(len(result[1]), round(3.3 * 32000))
        pre_roll = bytes(session.pre_roll)
        session.control({'type': 'session.cancel', 'sessionId': identity})
        self.assertEqual(session.pre_roll, pre_roll)
        wake.reset.assert_not_called()

    def test_announcement_processing_retains_each_preroll_frame_once(self):
        wake = self.detector()
        session = SatelliteSession(wake, early_wake=True)
        session.start_announcement({'id': 1, 'attempt': 0, 'text': 'test', 'purpose': 'announcement'})
        for i in range(150):
            session.accept_stereo(frame(i))
        mono = lambda stereo: b''.join(stereo[i:i+2] for i in range(0, len(stereo), 4))
        self.assertEqual(bytes(session.pre_roll), b''.join(mono(frame(i)) for i in range(150)))

    def test_busy_detection_interrupts_then_starts_a_prefix_checked_wake(self):
        for phase in ('wake', 'command', 'confirming', 'processing', 'playing', 'echo_guard', 'conversation'):
            wake = self.detector()
            session = SatelliteSession(wake, early_wake=True)
            session.phase = phase
            session.session_id = 'a' * 32
            wake.process.return_value = SimpleNamespace(name='Hey Orion', score=.7)
            events = session.accept_stereo(frame())
            self.assertEqual([e['type'] for e in events], ['session.interrupted', 'wake.candidate'])
            self.assertEqual(events[0]['sessionId'], 'a' * 32)
            self.assertNotEqual(events[1]['sessionId'], 'a' * 32)
            self.assertFalse(events[1]['acousticVerification'])
            self.assertTrue(session.prefix_pending)
