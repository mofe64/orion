import unittest
from types import SimpleNamespace
import numpy as np
from orion_voice.satellite import SatelliteSession, StereoCapture
from orion_voice.direction import DirectionEstimator


def frame(value=2000):
    return np.full((320,2), value, dtype='<i2').tobytes()

class Wake:
    next = False
    def reset(self): self.next = False
    def process(self, _):
        if self.next:
            self.next = False
            return SimpleNamespace(name='hey_orion', score=.7)

class SatelliteTests(unittest.TestCase):
    def setUp(self):
        self.now = 0
        self.wake = Wake()
        self.session = SatelliteSession(self.wake, clock=lambda:self.now)

    def trigger(self):
        self.session.accept_stereo(frame(1234))
        self.wake.next = True
        result = self.session.accept_stereo(frame())
        self.assertEqual(result[0]['type'], 'wake.candidate')
        return self.session.session_id

    def endpoint(self):
        result = []
        for _ in range(15): self.session.accept_stereo(frame())
        for _ in range(100):
            result = self.session.accept_stereo(frame(0))
            if result: return result
        self.fail('No endpoint')

    def test_pre_roll_preserves_wake_audio(self):
        self.trigger()
        result = self.endpoint()
        self.assertEqual(result[0]['purpose'], 'wake_and_command')
        self.assertEqual(np.frombuffer(result[1],dtype='<i2')[0],1234)

    def test_followup_during_confirmation_is_preserved(self):
        sid = self.trigger()
        self.endpoint()
        for _ in range(20): self.session.accept_stereo(frame(3000))
        for _ in range(60): self.session.accept_stereo(frame(0))
        result = self.session.control({'type':'wake.confirmed','sessionId':sid,'followup':True})
        self.assertEqual(result[0]['purpose'],'command')
        self.assertIn(3000,np.frombuffer(result[1],dtype='<i2'))

    def test_stale_control_does_not_cancel_current_turn(self):
        sid = self.trigger()
        with self.assertRaises(ValueError): self.session.control({'type':'session.finish','sessionId':'old'})
        self.assertEqual(self.session.session_id,sid)

    def test_processing_and_playback_ignore_wake(self):
        sid = self.trigger(); self.endpoint()
        self.session.control({'type':'wake.confirmed','sessionId':sid,'followup':False})
        self.session.control({'type':'session.playing','sessionId':sid})
        self.wake.next = True
        for _ in range(20): self.assertEqual(self.session.accept_stereo(frame()),[])
        self.session.control({'type':'session.finish','sessionId':sid})
        self.assertIsNone(self.session.session_id)
        self.assertFalse(self.wake.next)

    def test_deadline_discards_session(self):
        sid=self.trigger(); self.now=121
        self.assertEqual(self.session.accept_stereo(frame())[0]['type'],'session.expired')
        self.assertIsNone(self.session.session_id)
        self.assertEqual(len(self.session.pre_roll),0)

    def test_completion_cannot_rearm_before_playback(self):
        sid = self.trigger()
        with self.assertRaises(ValueError):
            self.session.control({'type':'session.finish','sessionId':sid})
        self.assertEqual(self.session.session_id,sid)

    def test_capture_is_stereo(self):
        self.assertEqual(StereoCapture().command()[-2:],['-c','2'])

    def test_reject_and_reset_clear_followup(self):
        sid=self.trigger(); self.endpoint()
        self.session.accept_stereo(frame())
        self.session.control({'type':'session.reject','sessionId':sid})
        self.assertEqual(self.session.followup,b'')

class DirectionTests(unittest.TestCase):
    def test_no_direction_without_commissioned_orientation(self):
        estimator=DirectionEstimator()
        estimator.accept(np.full((320,2),2000))
        self.assertEqual(estimator.observation()['side'],'unknown')

    def test_delayed_channels_and_swapped_orientation(self):
        rng=np.random.default_rng(71)
        for sign,expected in [(1,'left'),(-1,'right')]:
            estimator=DirectionEstimator(.06,sign)
            for _ in range(10):
                x=rng.normal(0,3000,320).astype(np.int16)
                estimator.accept(np.stack([x,np.roll(x,2)],axis=1))
            self.assertEqual(estimator.observation()['side'],expected)
            self.assertGreaterEqual(estimator.observation()['confidence'],.75)

    def test_silence_and_clipping_do_not_produce_direction(self):
        estimator=DirectionEstimator(.06,1)
        for _ in range(10): estimator.accept(np.full((320,2),32767,dtype=np.int16))
        self.assertEqual(estimator.observation()['side'],'unknown')
