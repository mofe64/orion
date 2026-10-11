import json
import time

from daemon_support import DaemonTestCase, daemon, request
from test_gaze import boottime_ns


class SurroundingsTests(DaemonTestCase):
    def test_presence_holds_expired_rest_until_privacy_clear(self):
        with daemon(hardware='v2', automatic_character=True, following=True,
                    gaze=False, rest_after_seconds=1) as path:
            self.wait_for_rest(path, 'awake')
            # Presence can defer rest even when gaze movement is disabled.
            deadline = time.monotonic() + 1.5
            while time.monotonic() < deadline:
                self.ok(path, 'gaze surroundings ' + json.dumps({
                    'capture_boottime_ns': boottime_ns(), 'camera_on': True,
                    'tracking_mode': 'faces', 'objects': [], 'looking_away': None,
                    'settings': {'presence_defers_rest': True},
                    'faces': [{'track_id': 1, 'azimuth': 0., 'elevation': 0., 'frontal': False}]}))
                time.sleep(.2)
            status = request(path, 'character status')['rest']
            self.assertEqual(status['state'], 'awake')
            self.assertEqual(status['remaining_seconds'], 0)
            self.assertTrue(status['presence_deferred'])
            self.assertIsNone(status['last_confirmed_at'])
            self.ok(path, 'gaze surroundings clear')
            status = self.wait_for_rest(path, 'going_to_rest')['rest']
            self.assertFalse(status['presence_deferred'])
            self.wait_for_rest(path, 'resting')
            self.assertFalse(request(path, 'status')['torque_enabled'])

    def test_metadata_freshness_voice_gate_and_stop_latch(self):
        with daemon(hardware='v2', automatic_character=True, following=True, gaze=True) as path:
            self.wait_for_rest(path, 'awake')
            self.assertTrue(request(path, 'gaze surroundings status')['ready'])
            stamp = boottime_ns()
            body = {'capture_boottime_ns': stamp, 'camera_on': True, 'tracking_mode': 'faces',
                'settings': {'object_glances': True}, 'looking_away': None,
                'objects': [{'id': 'object:1', 'category': 'book', 'azimuth': .4,
                    'elevation': -.3, 'capture_boottime_ns': str(stamp), 'point': None}]}
            command = 'gaze surroundings ' + json.dumps(body)
            self.ok(path, command)
            self.assertFalse(request(path, command)['ok'])  # No replaying timestamps.
            body['capture_boottime_ns'] = boottime_ns() - 2_000_000_000
            self.assertFalse(request(path, 'gaze surroundings ' + json.dumps(body))['ok'])
            self.voice(path, 'a'*32, 'wake')  # Even an unconfirmed wake closes background admission.
            self.assertFalse(request(path, 'gaze surroundings status')['ready'])
            self.voice(path, 'a'*32, 'cancel')
            request(path, 'stop')
            self.ok(path, 'gaze surroundings clear')
            self.assertFalse(request(path, 'gaze surroundings status')['ready'])
            self.assertFalse(request(path, 'gaze status')['gaze']['enabled'])

    def test_mutual_gaze_waits_for_dwell_and_yields_to_unconfirmed_wake(self):
        with daemon(hardware='v2', automatic_character=True, following=True, gaze=True) as path:
            self.wait_for_rest(path, 'awake')
            def observe():
                return 'gaze surroundings ' + json.dumps({
                    'capture_boottime_ns': boottime_ns(), 'camera_on': True, 'tracking_mode': 'faces',
                    'settings': {'mutual_gaze': True}, 'objects': [], 'looking_away': False,
                    'faces': [{'track_id': 1, 'azimuth': .06, 'elevation': 0., 'frontal': True}]})
            self.ok(path, observe())
            self.assertNotEqual(request(path, 'character status')['character'].get('active_clip'), 'mutual_gaze_notice')
            for _ in range(7):
                time.sleep(.25)
                self.ok(path, observe())
            self.assertEqual(request(path, 'character status')['character'].get('active_clip'), 'mutual_gaze_notice')
            anchor = request(path, 'character status')['character']['active_anchor']
            self.voice(path, 'b'*32, 'wake')
            self.ok(path, observe())
            self.assertFalse(request(path, 'gaze surroundings status')['ready'])
            self.wait_for(path, 'character status', lambda status: not (status['character'].get('active_clip') or '').startswith('mutual_gaze'))
            self.assertEqual(request(path, 'character status')['character']['active_anchor'], anchor)

    def test_novelty_schema_dwell_and_privacy_clear_keep_the_character_anchor(self):
        with daemon(hardware='v2', automatic_character=True, following=True, gaze=True) as path:
            self.wait_for_rest(path, 'awake')
            anchor = request(path, 'character status')['character']['active_anchor']
            def observe(dwell, category='book'):
                stamp = boottime_ns()
                return 'gaze surroundings ' + json.dumps({
                    'capture_boottime_ns': stamp, 'camera_on': True, 'tracking_mode': 'faces',
                    'settings': {'new_object_peek': True}, 'objects': [], 'looking_away': None,
                    'novelty': {'id': 'object:1', 'interest_key': 'object:book:0:-2',
                        'category': category, 'azimuth': .06, 'elevation': -.6,
                        'point': [0, -.28, 0], 'capture_boottime_ns': stamp,
                        'since_boottime_ns': stamp - dwell}})
            self.assertFalse(request(path, observe(3_000_000_000, 'person'))['ok'])
            self.ok(path, observe(1_000_000_000))
            self.assertNotEqual(request(path, 'character status')['character'].get('active_clip'), 'peek_new_object')
            self.ok(path, observe(3_000_000_000))
            self.assertEqual(request(path, 'character status')['character'].get('active_clip'), 'peek_new_object')
            self.ok(path, 'gaze surroundings clear')
            self.wait_for(path, 'character status', lambda status: status['character'].get('active_clip') != 'peek_new_object')
            self.assertEqual(request(path, 'character status')['character']['active_anchor'], anchor)
