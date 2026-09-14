from pathlib import Path
import time
import unittest
import uuid
import wave

from daemon_support import DaemonTestCase, daemon, request


class RestDaemonTests(DaemonTestCase):
    def test_idle_rest_darkness_torque_release_and_confirmed_wake_on_real_daemon(self):
        with daemon(automatic_character=True, rest_after_seconds=2, following=True) as path:
            self.wait_for_rest(path, 'awake')
            self.assertTrue(request(path, 'lamp 80 30 10 80')['ok'])
            self.wait_for_rest(path, 'resting')
            self.assertFalse(request(path, 'status')['torque_enabled'])

            self.assertFalse(request(path, 'character status')['rest']['light_on'])
            session = 'c' * 32
            for command in ['lamp 255 255 255 255', f'voice {session} wake', f'voice {session} endpoint']:
                self.assertTrue(request(path, command)['ok'])
            time.sleep(.1)
            self.assertEqual(request(path, 'character status')['rest']['state'], 'resting')
            self.assertFalse(request(path, 'character status')['rest']['light_on'])
            self.assertFalse(request(path, 'status')['torque_enabled'])
            self.assertTrue(request(path, f'voice {session} confirmed')['ok'])
            self.assertTrue(request(path, f'voice {session} followup')['ok'])
            status = self.wait_for_rest(path, 'awake')
            self.assertEqual(status['character']['state'], 'listening')
            self.assertTrue(request(path, 'status')['torque_enabled'])
            time.sleep(2.2)
            self.assertEqual(request(path, 'character status')['rest']['state'], 'awake')
            request(path, f'voice {session} finish')
            self.wait_for_rest(path, 'resting')
            self.assertFalse(request(path, 'status')['torque_enabled'])

    def test_native_mujoco_rest_contact_timeout_retains_torque_and_reports_fault(self):
        with daemon(automatic_character=True, rest_after_seconds=2) as path:
            self.wait_for_rest(path, 'awake')
            self.wait_for_rest(path, 'fault')
            runtime = request(path, 'status')
            self.assertEqual(runtime['last_motion']['name'], 'rest')
            self.assertEqual(runtime['last_motion']['state'], 'timed_out')
            self.assertGreater(runtime['last_motion']['max_position_error_rad'], 0.05)
            self.assertTrue(runtime['torque_enabled'])
            self.assertIn('torque has not been released', request(path, 'character status')['rest']['error'])

    def test_cancelled_reply_stays_cancelled_after_waking(self):
        with daemon(automatic_character=True, following=True) as path:
            self.wait_for_rest(path, 'awake')
            request(path, 'character rest')
            self.wait_for_rest(path, 'resting')
            session = 'd' * 32
            for event in ['wake', 'endpoint', 'confirmed']:
                self.assertTrue(request(path, f'voice {session} {event}')['ok'])
            spool = Path('/tmp/orion-speech-spool')
            spool.mkdir(exist_ok=True)
            identifier = 'rest-test-' + uuid.uuid4().hex
            wav_path = spool / f'{identifier}.wav'
            try:
                with wave.open(str(wav_path), 'wb') as wav:
                    wav.setnchannels(1); wav.setsampwidth(2); wav.setframerate(24000)
                    wav.writeframes(b'\0\0' * 24000)
                response = request(path, f'speech file {identifier} {session}')
                self.assertTrue(response['ok'])
                self.assertEqual(request(path, 'speech status')['speech']['state'], 'queued')
                request(path, f'voice {session} cancel')
                self.wait_for_rest(path, 'awake')
                last = request(path, 'speech status')['last_speech']
                self.assertEqual(last['run_id'], response['run_id'])
                self.assertEqual(last['state'], 'cancelled')
                self.assertIsNone(last['first_playback_ms'])
                self.assertFalse(wav_path.exists())
            finally:
                wav_path.unlink(missing_ok=True)

    def test_reply_upload_cannot_race_asr_confirmation_while_awake(self):
        with daemon(automatic_character=True, following=True) as path:
            self.wait_for_rest(path, 'awake')
            session = 'e' * 32
            request(path, f'voice {session} wake')
            request(path, f'voice {session} endpoint')
            spool = Path('/tmp/orion-speech-spool'); spool.mkdir(exist_ok=True)
            identifier = 'confirm-test-' + uuid.uuid4().hex
            wav_path = spool / f'{identifier}.wav'
            try:
                with wave.open(str(wav_path), 'wb') as wav:
                    wav.setnchannels(1); wav.setsampwidth(2); wav.setframerate(24000)
                    wav.writeframes(b'\0\0' * 24000)
                self.assertTrue(request(path, f'speech file {identifier} {session}')['ok'])
                time.sleep(.1)
                self.assertEqual(request(path, 'speech status')['speech']['state'], 'queued')
                self.assertTrue(request(path, f'voice {session} confirmed')['ok'])
                deadline = time.monotonic() + 3
                while time.monotonic() < deadline:
                    status = request(path, 'speech status')
                    if status['last_speech'] is not None: break
                    time.sleep(.02)
                self.assertEqual(status['last_speech']['state'], 'completed')
                self.assertIsNotNone(status['last_speech']['first_playback_ms'])
            finally:
                wav_path.unlink(missing_ok=True)

    def test_candidate_at_deadline_can_confirm_during_descent_without_torque_cycle(self):
        with daemon(automatic_character=True, rest_after_seconds=1, following=True) as path:
            self.wait_for_rest(path, 'awake')
            session = 'a' * 32
            self.voice(path, session, 'wake', 'endpoint')
            descent = self.wait_for_rest(path, 'going_to_rest')
            run = descent['rest']['movement_run_id']
            self.assertIsNone(descent['rest']['last_confirmed_at'])
            self.assertEqual(request(path, 'voice status')['voice']['session'], session)
            self.voice(path, session, 'confirmed')
            self.assertEqual(request(path, 'character status')['rest']['movement_run_id'], run)
            awake = self.wait_for_rest(path, 'awake')
            self.assertEqual(awake['character']['state'], 'thinking')
            self.assertNotIn('deactivate', self.torque_events(path))

    def test_cancelled_confirmation_during_descent_leaves_robot_resting(self):
        with daemon(automatic_character=True, following=True) as path:
            self.wait_for_rest(path, 'awake')
            self.ok(path, 'character rest')
            self.voice(path, 'a' * 32, 'wake', 'endpoint', 'confirmed', 'cancel')
            self.wait_for_rest(path, 'resting')
            self.assertFalse(request(path, 'status')['torque_enabled'])
            self.assertEqual(self.torque_events(path).count('deactivate'), 1)
            self.assertFalse(request(path, 'character status')['character']['enabled'])

    def test_foreground_motion_defers_expired_rest_until_completion(self):
        with daemon(automatic_character=True, rest_after_seconds=1, following=True) as path:
            self.wait_for_rest(path, 'awake')
            run = self.ok(path, 'goto attentive 3.0')['run_id']
            status = self.wait_for(path, 'character status',
                                   lambda s: s['rest']['remaining_seconds'] == 0)
            self.assertEqual(status['rest']['state'], 'awake')
            self.assertEqual(request(path, 'status')['motion']['run_id'], run)
            self.wait_for_rest(path, 'going_to_rest')
            last = request(path, 'status')['last_motion']
            self.assertEqual((last['run_id'], last['state']), (run, 'completed'))
            self.wait_for_rest(path, 'resting')

    def test_continuation_defers_rest_without_resetting_confirmed_deadline(self):
        with daemon(automatic_character=True, rest_after_seconds=2, following=True) as path:
            self.wait_for_rest(path, 'awake')
            self.voice(path, 'a' * 32, 'wake', 'endpoint', 'confirmed')
            confirmed = request(path, 'character status')['rest']['last_confirmed_at']
            self.voice(path, 'a' * 32, 'finish')
            self.voice(path, 'b' * 32, 'continue', 'endpoint')
            status = self.wait_for(path, 'character status',
                                   lambda s: s['rest']['remaining_seconds'] == 0)
            self.assertEqual(status['rest']['state'], 'awake')
            self.assertEqual(status['rest']['last_confirmed_at'], confirmed)
            self.voice(path, 'b' * 32, 'finish')
            self.wait_for_rest(path, 'resting')

    def test_attention_freshness_is_evaluated_after_home(self):
        for age in (0, 2999):
            with self.subTest(age=age), daemon(automatic_character=True, following=True) as path:
                self.wait_for_rest(path, 'awake')
                self.ok(path, 'character rest')
                self.wait_for_rest(path, 'resting')
                session = 'a' * 32
                self.voice(path, session, 'wake', 'endpoint', 'confirmed',
                           f'attend_left {age}', 'followup')
                awake = self.wait_for_rest(path, 'awake')
                if age == 0:
                    self.wait_for(path, 'status', lambda s: s['last_motion'] is not None
                                  and s['last_motion']['name'] == 'attention_left'
                                  and s['last_motion']['state'] == 'completed')
                    status = self.wait_for(path, 'character status', lambda s:
                                          s['character']['active_anchor']['base_yaw_joint'] < -0.3)
                else:
                    status = awake
                    self.assertAlmostEqual(status['character']['active_anchor']['base_yaw_joint'], 0, delta=.05)
                    self.assertNotEqual(status['character'].get('active_clip'), 'attention_left')
                self.assertEqual(status['character']['state'], 'listening')

    def test_reply_waits_for_home_and_attention_then_plays(self):
        with daemon(automatic_character=True, following=True) as path, self.reply_file() as (identifier, wav):
            self.wait_for_rest(path, 'awake')
            self.ok(path, 'character rest')
            self.wait_for_rest(path, 'resting')
            session = 'a' * 32
            self.voice(path, session, 'wake', 'endpoint', 'confirmed', 'attend_left 0')
            run = self.ok(path, f'speech file {identifier} {session}')['run_id']
            self.assertEqual(request(path, 'status')['motion']['name'], 'home')
            self.assertEqual(request(path, 'speech status')['speech']['state'], 'queued')
            self.wait_for(path, 'status', lambda s: s['motion'] is not None
                          and s['motion']['name'] == 'attention_left')
            self.assertEqual(request(path, 'speech status')['speech']['state'], 'queued')
            result = self.wait_for(path, 'speech status', lambda s: s['last_speech'] is not None)
            self.assertEqual((result['last_speech']['run_id'], result['last_speech']['state']),
                             (run, 'completed'))
            self.assertIsNotNone(result['last_speech']['first_playback_ms'])
            self.assertFalse(wav.exists())

    def test_failed_home_cancels_queued_reply_and_blocks_attention(self):
        with daemon(automatic_character=True, following=True) as path, self.reply_file() as (identifier, wav):
            self.wait_for_rest(path, 'awake')
            self.ok(path, 'character rest')
            self.wait_for_rest(path, 'resting')
            self.driver_control(path, stall=True)
            session = 'a' * 32
            self.voice(path, session, 'wake', 'endpoint', 'confirmed', 'attend_right 0')
            self.ok(path, f'speech file {identifier} {session}')
            fault = self.wait_for_rest(path, 'fault')
            self.assertIn('Home startup', fault['rest']['error'])
            status = request(path, 'status')
            self.assertTrue(status['torque_enabled'])
            self.assertEqual((status['last_motion']['name'], status['last_motion']['state']),
                             ('home', 'timed_out'))
            speech = self.wait_for(path, 'speech status', lambda s: s['last_speech'] is not None)['last_speech']
            self.assertEqual(speech['state'], 'cancelled')
            self.assertIsNone(speech['first_playback_ms'])
            self.assertFalse(wav.exists())
            self.assertNotEqual(fault['character'].get('active_clip'), 'attention_right')

    def test_torque_release_failure_reports_fault_and_refuses_automatic_wake(self):
        with daemon(automatic_character=True, following=True) as path:
            self.wait_for_rest(path, 'awake')
            self.driver_control(path, release_fails=True)
            self.ok(path, 'character rest')
            self.wait_for_rest(path, 'fault')
            runtime = request(path, 'status')
            self.assertEqual(runtime['last_motion']['state'], 'completed')
            self.assertTrue(runtime['torque_enabled'])
            self.voice(path, 'a' * 32, 'wake', 'endpoint', 'confirmed')
            self.assertEqual(request(path, 'character status')['rest']['state'], 'fault')
            self.assertEqual(self.torque_events(path).count('deactivate'), 1)

    def test_maintenance_and_explicit_stop_do_not_automatically_wake(self):
        for started in (False, True):
            with self.subTest(started=started), daemon(automatic_character=started, following=True) as path:
                if started:
                    self.wait_for_rest(path, 'awake')
                    self.ok(path, 'character stop')
                    self.wait_for_character(path, 'off', False)
                self.voice(path, 'a' * 32, 'wake', 'endpoint', 'confirmed')
                status = request(path, 'character status')
                self.assertEqual(status['rest']['state'], 'disabled')
                self.assertFalse(status['character']['enabled'])
                self.assertIsNone(request(path, 'status')['motion'])


if __name__ == '__main__':
    unittest.main(verbosity=2)
