import json

from daemon_support import DaemonTestCase, daemon, request
from test_gaze import boottime_ns


class SurroundingsTests(DaemonTestCase):
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
