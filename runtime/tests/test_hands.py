"""Hands use the same bearing command and gaze lifecycle as Faces."""
import json
import time
from daemon_support import DaemonTestCase, daemon, request
from test_gaze import boottime_ns


class HandsTests(DaemonTestCase):
    def observe(self, path, mode='hands', count=1):
        self.ok(path, 'gaze surroundings ' + json.dumps(dict(
            capture_boottime_ns=boottime_ns(), camera_on=True, tracking_mode=mode,
            settings={}, objects=[], faces=[], looking_away=None, hand_count=count)))

    def target(self, path):
        return request(path, 'gaze target ' + json.dumps(dict(
            track_id=1, capture_boottime_ns=boottime_ns(), azimuth=.3,
            elevation=.5, confidence=.95, **{'class': 'hand'})))

    def test_hands_use_gaze_and_stop_remains_explicit(self):
        with daemon(hardware='v2', automatic_character=True, following=True, gaze=True) as path:
            original = self.wait_for_rest(path, 'awake')['character']['active_anchor']
            self.observe(path, count=0)
            for _ in range(8):
                self.assertTrue(self.target(path)['accepted'])
                time.sleep(.1)
            self.assertEqual(request(path, 'character status')['character']['active_anchor'], original)
            self.assertEqual(request(path, 'character status')['character']['active_clip'], 'gaze_performance')
            self.assertNotIn('hands_failure', request(path, 'character status')['character'])
            self.ok(path, 'stop')
            self.observe(path)
            self.assertFalse(self.target(path).get('accepted', False))
            self.assertFalse(request(path, 'gaze status')['gaze']['enabled'])

    def test_target_loss_and_mode_change_use_gaze_clear(self):
        with daemon(hardware='v2', automatic_character=True, following=True, gaze=True) as path:
            original = self.wait_for_rest(path, 'awake')['character']['active_anchor']
            for _ in range(5):
                self.observe(path)
                self.assertTrue(self.target(path)['accepted'])
                time.sleep(.1)
            self.observe(path, mode='idle', count=0)
            self.ok(path, 'gaze clear')
            self.wait_for(path, 'gaze status', lambda v: v['gaze']['run_id'] is None)
            self.assertEqual(request(path, 'character status')['character']['active_anchor'], original)
            self.assertTrue(request(path, 'gaze surroundings status')['ready'])
