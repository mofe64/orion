import json
import time
from daemon_support import DaemonTestCase, daemon, request
from test_gaze import boottime_ns


class HandsTests(DaemonTestCase):
    def test_hands_metadata_enters_work_pose_and_privacy_recovers_after_voice(self):
        with daemon(hardware='v2', automatic_character=True, following=True, gaze=True) as path:
            self.wait_for_rest(path, 'awake')
            original=request(path,'character status')['character']['active_anchor']
            def observe():
                return 'gaze surroundings '+json.dumps(dict(capture_boottime_ns=boottime_ns(),
                    camera_on=True,tracking_mode='hands',settings={},objects=[],faces=[],looking_away=None,
                    hand_count=1,hand_point=None))
            self.ok(path,observe())
            self.assertEqual(request(path,'character status')['character'].get('active_clip'),'hands_entry')
            deadline=time.monotonic()+10
            while time.monotonic()<deadline:
                time.sleep(.3);self.ok(path,observe())
                status=request(path,'character status')['character']
                if status.get('active_clip')!='hands_entry':break
            self.assertAlmostEqual(status['active_anchor']['head_pitch_joint'],.4)
            self.assertFalse(request(path,'gaze surroundings status')['ready'])
            self.voice(path,'c'*32,'wake')
            self.ok(path,'gaze surroundings clear')
            self.assertEqual(request(path,'character status')['character']['active_anchor'],status['active_anchor'])
            self.voice(path,'c'*32,'cancel')
            self.ok(path,'gaze hands off')
            self.wait_for(path,'character status',lambda value:value['character']['active_anchor']==original)
            request(path,'stop')
            self.ok(path,observe())
            self.assertFalse(request(path,'gaze status')['gaze']['enabled'])
            self.assertNotEqual(request(path,'character status')['character'].get('active_clip'),'hands_entry')

    def test_idle_mode_leaves_hands_and_restores_the_previous_anchor(self):
        with daemon(hardware='v2', automatic_character=True, following=True, gaze=True) as path:
            original = self.wait_for_rest(path, 'awake')['character']['active_anchor']
            def observe_hands():
                self.ok(path, 'gaze surroundings ' + json.dumps(dict(
                    capture_boottime_ns=boottime_ns(), camera_on=True, tracking_mode='hands',
                    settings={}, objects=[], faces=[], looking_away=None, hand_count=1, hand_point=None)))
            observe_hands()
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                time.sleep(.3); observe_hands()
                status = request(path, 'character status')['character']
                if status.get('active_clip') != 'hands_entry':
                    break
            self.assertAlmostEqual(status['active_anchor']['head_pitch_joint'], .4)
            self.ok(path, 'gaze surroundings ' + json.dumps(dict(
                capture_boottime_ns=boottime_ns(), camera_on=True, tracking_mode='idle',
                settings={}, objects=[], faces=[], looking_away=None)))
            self.wait_for(path, 'character status', lambda value:
                value['character']['active_anchor'] == original)
            self.assertTrue(request(path, 'gaze surroundings status')['ready'])
