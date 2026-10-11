import ctypes
import json
from pathlib import Path
import sys
import tempfile
import time

from daemon_support import ROOT, DaemonTestCase, daemon, request


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
                self.assertEqual(request(path, 'gaze status')['gaze']['state'], 'stopped')
                for _ in range(7):
                    response = request(path, gaze_target())
                    self.assertFalse(response.get('accepted', False), response)
                    self.assertIsNone(request(path, 'gaze status')['gaze']['run_id'])
                    self.assertIsNone(request(path, 'status')['motion'])
                    time.sleep(.1)
                self.assertFalse(request(path, 'gaze status')['gaze']['enabled'])
                if command == 'disable':
                    rejected = request(path, 'gaze resume')
                    self.assertFalse(rejected['ok'], rejected)
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

    def test_defaults_enable_v2_serving_and_leave_v1_off(self):
        for hardware, following, enabled in (('v1', True, False), ('v2', True, True), ('v2', False, True)):
            with self.subTest(hardware=hardware, following=following), daemon(hardware=hardware, following=following) as path:
                self.assertEqual(request(path, 'gaze status')['gaze']['enabled'], enabled)
                self.assertEqual(request(path, 'status')['mode'], 'observe')

    def test_explicit_off_rejects_targets_attention_and_all_resume_requests(self):
        with daemon(hardware='v2', following=True, gaze=False) as path:
            self.assertFalse(request(path, 'gaze status')['gaze']['enabled'])
            self.assertFalse(request(path, gaze_target())['ok'])
            self.assertFalse(request(path, 'gaze resume')['ok'])
            self.ok(path, 'gaze clear')
            self.assertFalse(request(path, 'gaze target {"joints":{}}')['ok'])
            self.ok(path, 'character start')
            self.wait_for_rest(path, 'awake')
            self.voice(path, 'a'*32, 'wake', 'endpoint', 'confirmed')
            for command in (gaze_target(), f'voice {"a"*32} attend_bearing .4 0 0',
                            f'voice {"a"*32} attend_scan 0', 'gaze resume'):
                response = request(path, command)
                self.assertFalse(response['ok'], response)
                self.assertIn('Gaze is unavailable', response['error'])
            self.assertEqual(request(path, 'gaze status')['gaze']['state'], 'off')

    def test_missing_geometry_reports_unavailable_without_blocking_the_daemon(self):
        for broken_reference in (False, True):
            with self.subTest(broken_reference=broken_reference), tempfile.TemporaryDirectory() as temporary:
                catalog = Path(temporary)
                for name in ('motion', 'scenes', 'audio'):
                    (catalog / name).symlink_to(ROOT / name, target_is_directory=True)
                if broken_reference:
                    urdf = catalog / 'description/urdf/orion-v2.urdf'
                    urdf.parent.mkdir(parents=True)
                    source = (ROOT / 'description/urdf/orion-v2.urdf').read_text()
                    urdf.write_text(source.replace('<child link="camera_optical_frame"',
                                                   '<child link="missing_camera_reference"'))
                    self.assertNotEqual(urdf.read_text(), source)
                with daemon(hardware='v2', following=True, gaze=True, catalog_root=catalog) as path:
                    status = request(path, 'gaze status')['gaze']
                    self.assertEqual(status['state'], 'unavailable')
                    self.assertFalse(status['enabled'])
                    self.assertTrue(status['error'])
                    self.ok(path, 'character start')
                    self.wait_for_rest(path, 'awake')
                    self.voice(path, 'a'*32, 'wake', 'endpoint', 'confirmed')
                    self.ok(path, 'gaze clear')
                    for command in ('gaze resume', gaze_target(), f'voice {"a"*32} attend_bearing .4 0 0',
                                    'camera-rays ' + json.dumps(dict(capture_boottime_ns=boottime_ns(), rays=[[0, 0, 1]]))):
                        self.assertFalse(request(path, command)['ok'], command)
                    self.assertEqual(request(path, 'gaze status')['gaze']['state'], 'unavailable')

    def test_release_geometry_works_with_a_catalog_without_camera_frames(self):
        with tempfile.TemporaryDirectory() as temporary:
            catalog = Path(temporary)
            for name in ('motion', 'scenes', 'audio'):
                (catalog / name).symlink_to(ROOT / name, target_is_directory=True)
            with daemon(hardware='v2', following=True, catalog_root=catalog,
                        camera_urdf=ROOT / 'description/urdf/orion-v2.urdf') as path:
                status = request(path, 'gaze status')['gaze']
                self.assertNotEqual(status['state'], 'unavailable')
                self.assertIsNone(status['error'])

    def test_character_start_and_powered_stop_wake_resume_gaze(self):
        for stop in ('stop', 'disable'):
            for resume in (('character start', 'confirmed wake') if stop == 'stop' else ('character start',)):
                with self.subTest(stop=stop, resume=resume), daemon(hardware='v2', following=True,
                        automatic_character=True, gaze=True) as path:
                    self.wait_for_rest(path, 'awake')
                    self.assertTrue(request(path, gaze_target())['accepted'])
                    old_run = self.wait_for(path, 'gaze status', lambda s: s['gaze']['run_id'] is not None)['gaze']['run_id']
                    self.ok(path, stop)
                    if resume == 'character start':
                        self.ok(path, resume)
                    else:
                        self.voice(path, 'a'*32, 'wake', 'endpoint')
                        self.assertFalse(request(path, 'gaze status')['gaze']['enabled'])
                        self.voice(path, 'a'*32, 'confirmed')
                    self.assertIn('stop_command', request(path, 'gaze status')['gaze'])
                    self.assertTrue(request(path, 'gaze status')['gaze']['enabled'])
                    self.assertIsNone(request(path, 'gaze status')['gaze']['stop_command'])
                    self.assertTrue(request(path, 'status')['torque_enabled'])
                    self.wait_for_rest(path, 'awake')
                    self.assertTrue(request(path, gaze_target(-.4))['accepted'])
                    new_run = self.wait_for(path, 'gaze status', lambda s: s['gaze']['run_id'] is not None)['gaze']['run_id']
                    self.assertGreater(new_run, old_run)

    def test_confirmed_wake_after_disable_never_restores_torque_or_motion(self):
        with daemon(hardware='v2', following=True, automatic_character=True) as path:
            self.wait_for_rest(path, 'awake')
            self.assertTrue(request(path, gaze_target())['accepted'])
            self.wait_for(path, 'gaze status', lambda s: s['gaze']['run_id'] is not None)
            self.ok(path, 'disable')
            events = self.torque_events(path)
            self.assertEqual(events[-1], 'deactivate')
            self.voice(path, 'a'*32, 'wake', 'endpoint', 'confirmed')
            for _ in range(5):
                runtime = request(path, 'status')
                self.assertFalse(runtime['torque_enabled'])
                self.assertIsNone(runtime['motion'])
                self.assertFalse(request(path, gaze_target())['ok'])
                time.sleep(.1)
            self.assertEqual(self.torque_events(path), events)
            status = request(path, 'character status')
            self.assertEqual(status['rest']['state'], 'disabled')
            self.assertIsNone(status['rest']['last_confirmed_at'])
            self.assertEqual(request(path, 'gaze status')['gaze']['state'], 'stopped')
            self.assertEqual(request(path, 'gaze status')['gaze']['stop_command'], 'disable')
            self.assertFalse(request(path, 'gaze resume')['ok'])
            self.assertFalse(request(path, f'voice {"a"*32} attend_bearing .4 .1 0')['ok'])

    def test_stop_and_disable_record_the_latch_command_and_disable_stays_explicit(self):
        with daemon(hardware='v2', following=True, automatic_character=True) as path:
            self.wait_for_rest(path, 'awake')
            self.assertTrue(request(path, gaze_target())['accepted'])
            self.wait_for(path, 'gaze status', lambda s: s['gaze']['run_id'] is not None)
            self.ok(path, 'stop')
            self.assertEqual(request(path, 'gaze status')['gaze'].get('stop_command'), 'stop')
            self.ok(path, 'disable')
            self.ok(path, 'gaze clear')
            request(path, 'stop')  # No run remains; this cannot downgrade disable.
            self.assertEqual(request(path, 'gaze status')['gaze']['stop_command'], 'disable')
            self.ok(path, 'enable')
            self.voice(path, 'a'*32, 'wake', 'endpoint', 'confirmed')
            self.assertEqual(request(path, 'gaze status')['gaze']['state'], 'stopped')
            self.assertEqual(request(path, 'character status')['rest']['state'], 'disabled')
            self.assertIsNone(request(path, 'gaze status')['gaze']['run_id'])
            self.assertFalse(request(path, gaze_target())['ok'])
            self.ok(path, 'gaze resume')  # Explicit script recovery after explicit power restore.
            self.assertIsNone(request(path, 'gaze status')['gaze']['stop_command'])
            self.assertTrue(request(path, gaze_target())['accepted'])

    def test_disable_latch_requires_explicit_recovery_even_after_manual_enable(self):
        with daemon(hardware='v2', following=True, automatic_character=True) as path:
            self.wait_for_rest(path, 'awake')
            self.ok(path, 'disable')
            self.ok(path, 'enable')
            self.voice(path, 'a'*32, 'wake', 'endpoint', 'confirmed')
            self.assertEqual(request(path, 'gaze status')['gaze']['state'], 'stopped')
            self.assertEqual(request(path, 'character status')['rest']['state'], 'disabled')
            self.assertIsNone(request(path, 'gaze status')['gaze']['run_id'])
            self.assertFalse(request(path, gaze_target())['ok'])
            self.voice(path, 'a'*32, 'finish')
            self.ok(path, 'character start')
            self.wait_for_rest(path, 'awake')
            self.assertTrue(request(path, gaze_target())['accepted'])

    def test_clear_idle_reconnecting_targets_and_stale_wakes_cannot_clear_stop(self):
        with daemon(hardware='v2', following=True, automatic_character=True, gaze=True) as path:
            self.wait_for_rest(path, 'awake')
            stale = 'a'*32
            self.voice(path, stale, 'wake', 'endpoint', 'confirmed')
            self.assertTrue(request(path, gaze_target())['accepted'])
            self.wait_for(path, 'gaze status', lambda s: s['gaze']['run_id'] is not None)
            self.ok(path, 'stop')
            self.ok(path, 'gaze clear')
            self.voice(path, stale, 'wake', 'endpoint')
            self.assertFalse(request(path, f'voice {stale} confirmed')['ok'])
            for _ in range(12):
                # Each request reconnects, as a returning vision worker does.
                self.assertFalse(request(path, gaze_target())['ok'])
                self.assertIsNone(request(path, 'status')['motion'])
                time.sleep(.1)
            self.assertEqual(request(path, 'gaze status')['gaze']['state'], 'stopped')
            self.voice(path, 'b'*32, 'wake', 'verify')
            self.assertEqual(request(path, 'gaze status')['gaze']['state'], 'stopped')

    def test_confirmed_wake_resumes_without_preempting_a_scene(self):
        with daemon(hardware='v2', following=True, automatic_character=True, gaze=True) as path:
            self.wait_for_rest(path, 'awake')
            self.assertTrue(request(path, gaze_target())['accepted'])
            self.wait_for(path, 'gaze status', lambda s: s['gaze']['run_id'] is not None)
            self.ok(path, 'stop')
            scene = self.ok(path, 'scene start return_home')['run_id']
            run = request(path, 'status')['motion']['run_id']
            self.assertFalse(request(path, 'character start')['ok'])
            self.voice(path, 'a'*32, 'wake', 'endpoint', 'confirmed')
            self.assertEqual(request(path, 'scene status')['scene']['run_id'], scene)
            self.assertEqual(request(path, 'status')['motion']['run_id'], run)
            self.assertTrue(request(path, 'gaze status')['gaze']['enabled'])
            self.assertFalse(request(path, gaze_target())['accepted'])
            self.wait_for(path, 'scene status', lambda s: s.get('scene') is None)
            self.wait_for_rest(path, 'awake')
            self.assertTrue(request(path, gaze_target(-.4))['accepted'])
            self.wait_for(path, 'gaze status', lambda s: s['gaze']['run_id'] is not None)

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
