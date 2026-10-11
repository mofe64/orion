import json

from daemon_support import DaemonTestCase, daemon, request
from test_gaze import gaze_target


TOKEN = 'e' * 32


def work(path, action, **fields):
    return request(path, 'aim-light ' + json.dumps({'action': action, 'token': TOKEN, **fields}))


class AimLightTests(DaemonTestCase):
    def complete(self, path, result):
        self.assertTrue(result['ok'], result)
        run = result['run_id']
        value = self.wait_for(path, 'status', lambda s: s.get('last_motion')
            and s['last_motion']['run_id'] == run, timeout=14)
        self.assertEqual(value['last_motion']['state'], 'completed', value)

    def test_aim_is_calibrated_owned_and_changes_anchor_only_after_completion(self):
        with daemon(hardware='v2', following=True, automatic_character=True) as path:
            self.wait_for_rest(path, 'awake')
            old = request(path, 'character status')['character']['active_anchor']
            self.assertTrue(work(path, 'begin')['ok'])
            self.assertFalse(request(path, gaze_target())['accepted'])
            result = work(path, 'target', point=[0, -.3, 0], lean=.8)
            self.assertTrue(result['ok'], result)
            self.assertIn(result['status'], ('aimed', 'clamped'))
            self.assertEqual(request(path, 'character status')['character']['active_anchor'], old)
            self.complete(path, result)
            self.assertTrue(work(path, 'status')['committed'])
            new = request(path, 'character status')['character']['active_anchor']
            self.assertGreater(new['shoulder_pitch_joint'], old['shoulder_pitch_joint'] + .4)
            self.assertTrue(work(path, 'end')['ok'])
            self.assertFalse(request(path, gaze_target())['accepted'])

    def test_scan_stops_restore_old_anchor_and_unreachable_never_moves(self):
        with daemon(hardware='v2', following=True, automatic_character=True) as path:
            self.wait_for_rest(path, 'awake')
            old = request(path, 'character status')['character']['active_anchor']
            self.assertTrue(work(path, 'begin')['ok'])
            result = work(path, 'target', point=[0, .3, 0], lean=.8)
            self.assertEqual(result, {'ok': True, 'status': 'unreachable'})
            self.assertIsNone(request(path, 'status')['motion'])
            self.complete(path, work(path, 'scan', index=0))
            self.assertEqual(request(path, 'character status')['character']['active_anchor'], old)
            self.complete(path, work(path, 'end'))
            self.assertEqual(request(path, 'character status')['character']['active_anchor'], old)

    def test_stop_revokes_owner_and_never_adopts_cancelled_aim(self):
        with daemon(hardware='v2', following=True, automatic_character=True) as path:
            self.wait_for_rest(path, 'awake')
            old = request(path, 'character status')['character']['active_anchor']
            self.assertTrue(work(path, 'begin')['ok'])
            self.assertTrue(work(path, 'target', point=[0, -.3, 0], lean=.8)['ok'])
            self.ok(path, 'stop')
            self.assertFalse(work(path, 'end')['ok'])
            self.assertIsNone(request(path, 'status')['motion'])
            self.assertEqual(request(path, 'character status')['character']['active_anchor'], old)
            self.assertFalse(work(path, 'begin')['ok'])
            self.assertFalse(request(path, 'gaze status')['gaze']['enabled'])

    def test_voice_interval_requires_current_confirmed_owner_and_phase(self):
        with daemon(hardware='v2', following=True, automatic_character=True) as path:
            self.wait_for_rest(path, 'awake')
            session = 'b' * 32
            self.assertFalse(request(path, f'voice {session} vision_begin')['ok'])
            self.voice(path, session, 'wake')
            self.assertFalse(request(path, f'voice {session} vision_begin')['ok'])
            self.voice(path, session, 'endpoint', 'confirmed', 'vision_begin')
            self.assertEqual(request(path, 'voice status')['vision_session'], session)
            self.voice(path, session, 'processing')
            self.assertIsNone(request(path, 'voice status')['vision_session'])
