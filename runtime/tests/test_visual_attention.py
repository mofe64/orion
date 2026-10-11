import time

from daemon_support import DaemonTestCase, daemon, request


class VisualAttentionTests(DaemonTestCase):
    def test_opt_in_and_owned_confirmed_fresh_session_are_required(self):
        with daemon(hardware='v2', following=True) as path:
            for verb in ('attend_bearing .4 .1 0', 'attend_scan 0'):
                self.assertFalse(request(path, f'voice {"a"*32} {verb}')['ok'])
        with daemon(hardware='v2', automatic_character=True, following=True, gaze=True) as path:
            self.wait_for_rest(path, 'awake')
            session = 'a'*32
            self.voice(path, session, 'wake', 'endpoint')
            self.ok(path, f'voice {session} attend_bearing .4 .1 0')  # Unconfirmed: ignore.
            time.sleep(.1)
            self.assertNotEqual(request(path, 'character status')['character'].get('active_clip'), 'gaze_attention')
            self.voice(path, session, 'confirmed')
            for verb in ('attend_bearing NaN 0 0', 'attend_bearing 0 inf 0',
                         'attend_bearing .4 0 -1', 'attend_scan NaN', 'attend_bearing .4 0 0 extra'):
                self.assertFalse(request(path, f'voice {session} {verb}')['ok'])
            self.ok(path, f'voice {"b"*32} attend_bearing .4 .1 0')
            self.ok(path, f'voice {session} attend_bearing .4 .1 3000')
            time.sleep(.1)
            self.assertNotEqual(request(path, 'character status')['character'].get('active_clip'), 'gaze_attention')
            self.ok(path, f'voice {session} attend_bearing .4 .1 0')
            self.wait_for(path, 'status', lambda s: s['last_motion'] is not None
                and s['last_motion']['name'] == 'gaze_attention' and s['last_motion']['state'] == 'completed')
            self.wait_for(path, 'character status', lambda s: s['character']['active_anchor']['base_yaw_joint'] > .35)

    def test_visual_freshness_and_retired_ownership_are_rechecked_after_home(self):
        for age, retire in ((0, False), (2999, False), (0, True)):
            with self.subTest(age=age, retire=retire), daemon(hardware='v2', automatic_character=True, following=True, gaze=True) as path:
                home_anchor = self.wait_for_rest(path, 'awake')['character']['active_anchor']
                self.ok(path, 'character rest')
                self.wait_for_rest(path, 'resting')
                session = 'a'*32
                self.voice(path, session, 'wake', 'endpoint', 'confirmed', f'attend_bearing .4 .1 {age}')
                if retire:
                    self.voice(path, session, 'finish')
                awake = self.wait_for_rest(path, 'awake')
                if age == 0 and not retire:
                    self.wait_for(path, 'status', lambda s: s['last_motion'] is not None
                        and s['last_motion']['name'] == 'gaze_attention' and s['last_motion']['state'] == 'completed')
                    self.wait_for(path, 'character status', lambda s: s['character']['active_anchor']['base_yaw_joint'] > .35)
                else:
                    self.assertNotEqual(awake['character'].get('active_clip'), 'gaze_attention')
                    self.assertAlmostEqual(awake['character']['active_anchor']['base_yaw_joint'],
                        home_anchor['base_yaw_joint'], delta=.001)

    def test_no_face_scans_once_then_gives_up(self):
        with daemon(hardware='v2', automatic_character=True, following=True, gaze=True) as path:
            self.wait_for_rest(path, 'awake')
            session = 'a'*32
            self.voice(path, session, 'wake', 'endpoint', 'confirmed', 'attend_scan 0')
            done = self.wait_for(path, 'status', lambda s: s['last_motion'] is not None
                and s['last_motion']['name'] == 'gaze_look_around' and s['last_motion']['state'] == 'completed')
            run = done['last_motion']['run_id']
            for _ in range(3):
                self.ok(path, f'voice {session} attend_scan 0')
                time.sleep(.1)
            status = request(path, 'status')
            self.assertEqual(status['last_motion']['run_id'], run)
            self.assertTrue(status['motion'] is None or status['motion']['name'] != 'gaze_look_around')

    def test_stop_and_scene_preempt_visual_attention_without_restart(self):
        for command in ('stop', 'scene start return_home'):
            with self.subTest(command=command), daemon(hardware='v2', automatic_character=True, following=True, gaze=True) as path:
                self.wait_for_rest(path, 'awake')
                session = 'a'*32
                self.voice(path, session, 'wake', 'endpoint', 'confirmed', 'attend_bearing .4 .1 0')
                self.wait_for(path, 'status', lambda s: s['motion'] is not None and s['motion']['name'] == 'gaze_attention')
                self.ok(path, command)
                self.ok(path, f'voice {session} attend_bearing .4 .1 0')
                if command.startswith('scene'):
                    self.wait_for(path, 'scene status', lambda s: s.get('scene') is None)
                time.sleep(.25)
                motion = request(path, 'status')['motion']
                self.assertTrue(motion is None or motion['name'] != 'gaze_attention')

    def test_first_reply_waits_for_measured_visual_attention_completion(self):
        with daemon(hardware='v2', automatic_character=True, following=True, gaze=True) as path, self.reply_file() as (identifier, wav):
            self.wait_for_rest(path, 'awake')
            session = 'a'*32
            self.voice(path, session, 'wake', 'endpoint', 'confirmed', 'attend_bearing .4 .1 0')
            self.wait_for(path, 'status', lambda s: s['motion'] is not None and s['motion']['name'] == 'gaze_attention')
            run = self.ok(path, f'speech file {identifier} {session}')['run_id']
            self.assertEqual(request(path, 'speech status')['speech']['state'], 'queued')
            result = self.wait_for(path, 'speech status', lambda s: s['last_speech'] is not None)
            self.assertEqual((result['last_speech']['run_id'], result['last_speech']['state']), (run, 'completed'))
            self.assertIsNotNone(result['last_speech']['first_playback_ms'])
            self.assertFalse(wav.exists())
