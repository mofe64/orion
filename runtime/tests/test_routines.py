import time
from daemon_support import DaemonTestCase, daemon, request


class RoutinesDaemonTests(DaemonTestCase):
    def test_lamp_mode_stays_awake_and_alert_rings_at_rest_without_enabling_torque(self):
        with daemon(automatic_character=True, rest_after_seconds=1, following=True) as path:
            self.wait_for_rest(path, 'awake')
            self.ok(path, 'routines {"action":"set_mode","mode":"lamp"}')
            time.sleep(1.2)
            status = request(path, 'character status')
            self.assertEqual(status['rest']['state'], 'awake')
            self.assertIsNone(status['rest']['remaining_seconds'])
            self.ok(path, 'character rest')
            self.wait_for_rest(path, 'resting')
            self.ok(path, 'routines {"action":"timer","seconds":1,"label":"tea"}')
            self.wait_for(path, 'routines status', lambda value: value['routines']['ringing'])
            self.assertFalse(request(path, 'status')['torque_enabled'])
            self.voice(path, 'a'*32, 'cancel', 'finish')
            self.assertTrue(request(path, 'routines status')['routines']['ringing'])
            self.assertFalse(request(path, 'speech stream stale')['ok'])
            self.ok(path, 'routines {"action":"stop"}')
            self.assertFalse(request(path, 'routines status')['routines']['ringing'])
            self.assertEqual(request(path, 'character status')['rest']['state'], 'resting')

    def test_voice_sleep_waits_for_session_finish_and_retains_lamp_mode(self):
        with daemon(automatic_character=True, following=True) as path:
            self.wait_for_rest(path, 'awake')
            self.ok(path, 'routines {"action":"set_mode","mode":"lamp"}')
            sid = 'b'*32
            self.assertFalse(request(path, f'sleep {sid}')['ok'])
            self.voice(path, sid, 'wake', 'endpoint', 'confirmed')
            self.ok(path, f'sleep {sid}')
            time.sleep(.15)
            self.assertEqual(request(path, 'character status')['rest']['state'], 'awake')
            self.voice(path, sid, 'finish')
            self.wait_for_rest(path, 'resting')
            self.assertEqual(request(path, 'routines status')['routines']['mode'], 'lamp')
