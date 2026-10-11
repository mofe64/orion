import ctypes
import json
import sys
import time

from daemon_support import DaemonTestCase, daemon, request


def boottime_ns():
    if sys.platform == 'linux':
        return time.clock_gettime_ns(time.CLOCK_BOOTTIME)
    if sys.platform == 'darwin':
        class Timebase(ctypes.Structure):
            _fields_ = [('numerator', ctypes.c_uint32), ('denominator', ctypes.c_uint32)]
        system = ctypes.CDLL(None)
        system.mach_continuous_time.restype = ctypes.c_uint64
        system.mach_timebase_info.argtypes = [ctypes.POINTER(Timebase)]
        info = Timebase()
        if system.mach_timebase_info(ctypes.byref(info)) != 0:
            raise AssertionError('Cannot read Darwin continuous clock scale')
        return system.mach_continuous_time() * info.numerator // info.denominator
    raise AssertionError('No suspend-aware test clock on this platform')


def gaze_target(azimuth=.4):
    return 'gaze target ' + json.dumps(dict(track_id=1, capture_boottime_ns=boottime_ns(),
        azimuth=azimuth, elevation=0.0, confidence=.95))


class GazeDaemonTests(DaemonTestCase):
    def test_stop_and_disable_latch_off_until_explicit_gaze_resume(self):
        for command in ('stop', 'disable'):
            with self.subTest(command=command), daemon(hardware='v2', following=True,
                    automatic_character=True, gaze=True) as path:
                self.wait_for_rest(path, 'awake')
                self.assertTrue(request(path, gaze_target())['accepted'])
                self.wait_for(path, 'gaze status', lambda s: s['gaze']['state'] == 'tracking')
                self.ok(path, command)
                for _ in range(7):
                    response = request(path, gaze_target())
                    self.assertFalse(response.get('accepted', False), response)
                    self.assertIsNone(request(path, 'gaze status')['gaze']['run_id'])
                    self.assertIsNone(request(path, 'status')['motion'])
                    time.sleep(.1)
                self.assertFalse(request(path, 'gaze status')['gaze']['enabled'])
                if command == 'disable':
                    self.ok(path, 'enable')
                    self.assertFalse(request(path, gaze_target()).get('accepted', False))
                self.ok(path, 'gaze resume')
                self.assertTrue(request(path, gaze_target())['accepted'])
                self.wait_for(path, 'gaze status', lambda s: s['gaze']['state'] == 'tracking')

    def test_status_distinguishes_compiled_peak_from_planning_steps(self):
        with daemon(hardware='v2', following=True, gaze=True) as path:
            status = request(path, 'gaze status')['gaze']
            self.assertEqual(status['peak_speed_rad_s'], 1.5)
            self.assertEqual(status['base_step_speed_rad_s'], .6)
            self.assertEqual(status['neck_step_speed_rad_s'], 2.5)

    def test_default_off_rejects_targets_and_status_and_clear_remain_available(self):
        with daemon(hardware='v2', following=True) as path:
            self.assertFalse(request(path, 'gaze status')['gaze']['enabled'])
            self.assertFalse(request(path, gaze_target())['ok'])
            self.assertFalse(request(path, 'gaze resume')['ok'])
            self.ok(path, 'gaze clear')
            self.assertFalse(request(path, 'gaze target {"joints":{}}')['ok'])

    def test_scene_and_stop_win_without_gaze_error_or_automatic_restart(self):
        with daemon(hardware='v2', following=True, automatic_character=True, gaze=True) as path:
            self.wait_for_rest(path, 'awake')
            self.assertTrue(request(path, gaze_target())['accepted'])
            self.wait_for(path, 'gaze status', lambda s: s['gaze']['state'] == 'tracking')
            self.ok(path, 'scene start return_home')
            self.assertFalse(request(path, gaze_target())['accepted'])
            self.assertIsNone(request(path, 'gaze status')['gaze']['error'])
            self.wait_for(path, 'scene status', lambda s: s.get('scene') is None)
            self.wait_for(path, 'character status', lambda s: s['character']['state'] in ('home_idle', 'pose_idle'))
            self.assertTrue(request(path, gaze_target())['accepted'])
            self.wait_for(path, 'gaze status', lambda s: s['gaze']['state'] == 'tracking')
            self.ok(path, 'stop')
            self.wait_for(path, 'gaze status', lambda s: s['gaze']['run_id'] is None)
            time.sleep(.25)
            self.assertIsNone(request(path, 'gaze status')['gaze']['track_id'])
            self.assertIsNone(request(path, 'status')['motion'])
