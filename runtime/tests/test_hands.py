import json
import time
from daemon_support import DaemonTestCase, daemon, request
from test_gaze import boottime_ns


class HandsTests(DaemonTestCase):
    def observe(self, path, mode='hands', point=None):
        self.ok(path, 'gaze surroundings ' + json.dumps(dict(
            capture_boottime_ns=boottime_ns(), camera_on=True, tracking_mode=mode,
            settings={}, objects=[], faces=[], looking_away=None,
            hand_count=int(point is not None), hand_point=point)))

    def test_hands_keeps_the_held_anchor_and_privacy_returns_after_voice(self):
        with daemon(hardware='v2', automatic_character=True, following=True, gaze=True) as path:
            original = self.wait_for_rest(path, 'awake')['character']['active_anchor']
            for _ in range(8):
                self.observe(path)
                time.sleep(.2)
            self.assertEqual(request(path, 'character status')['character']['active_anchor'], original)
            self.assertFalse(request(path, 'gaze surroundings status')['ready'])
            self.voice(path, 'c' * 32, 'wake')
            self.ok(path, 'gaze surroundings clear')
            self.voice(path, 'c' * 32, 'cancel')
            self.ok(path, 'gaze hands off')
            self.wait_for(path, 'character status', lambda v:
                v['character'].get('active_clip') is None and v['character'].get('hands_failure') is None)
            self.assertEqual(request(path, 'character status')['character']['active_anchor'], original)
            request(path, 'stop')
            self.observe(path, point=[0.08, -.35, 0.])
            self.assertFalse(request(path, 'gaze status')['gaze']['enabled'])
            self.assertNotEqual(request(path, 'character status')['character'].get('active_clip'), 'hands_track')

    def test_idle_mode_releases_hands_with_the_original_anchor(self):
        with daemon(hardware='v2', automatic_character=True, following=True, gaze=True) as path:
            original = self.wait_for_rest(path, 'awake')['character']['active_anchor']
            for _ in range(8):
                self.observe(path, point=[0.08, -.35, 0.])
                time.sleep(.2)
            self.assertEqual(request(path, 'character status')['character']['active_anchor'], original)
            self.observe(path, mode='idle')
            self.wait_for(path, 'character status', lambda v:
                v['character'].get('active_clip') is None and v['character'].get('hands_failure') is None)
            self.assertEqual(request(path, 'character status')['character']['active_anchor'], original)
            self.assertTrue(request(path, 'gaze surroundings status')['ready'])
