import json
import re
from pathlib import Path
import tempfile
import unittest
from unittest.mock import ANY, Mock, patch

from test_gateway import OrionGateway, GatewayError
from gateway import AIM_TOTAL_SECONDS, AIM_RECOVERY_SECONDS, AIM_VOICE_END_SECONDS, FIND_TIMEOUT_SECONDS, UnixOrionClient


class AimGatewayTests(unittest.TestCase):
    def test_timing_budget_covers_recovery_and_precedes_coordinator_release(self):
        root = Path(__file__).resolve().parents[2]
        coordinator = (root / 'coordinator/src/gateway.rs').read_text()
        runtime = (root / 'runtime/src/expression/aim_light.rs').read_text()
        voice_timeout = float(re.search(r'AIM_LIGHT_TIMEOUT_SECONDS: u64 = (\d+)', coordinator)[1])
        recovery_lease = float(re.search(r'END_RECOVERY_LEASE_SECONDS: f64 = ([\d.]+)', runtime)[1])
        self.assertLessEqual(AIM_TOTAL_SECONDS + 5, voice_timeout)
        self.assertGreaterEqual(AIM_RECOVERY_SECONDS, recovery_lease + 2 + AIM_VOICE_END_SECONDS + 2)
        self.assertLess(FIND_TIMEOUT_SECONDS, AIM_TOTAL_SECONDS - AIM_RECOVERY_SECONDS)

    def gateway(self, finds, aimed='aimed'):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        gateway = OrionGateway(Mock(), hardware='v2', camera_preference_file=Path(temporary.name) / 'camera.json')
        commands = []
        state = {'run': 0, 'session': None, 'interval': None, 'committed': False}
        def checked(command, **kwargs):
            commands.append(command)
            if command == 'voice status':
                return {'ok': True, 'voice': {'session': state['session']}, 'vision_session': state['interval']}
            if command.startswith('voice '):
                _, session, event = command.split()
                state['session'] = session
                state['interval'] = session if event == 'vision_begin' else None
                return {'ok': True}
            if command == 'status':
                # Fresh gaze can begin as a restoring end relinquishes ownership.
                return {'ok': True, 'motion': {'run_id': 999, 'state': 'executing'},
                        'last_motion': {'run_id': state['run'], 'state': 'completed'}}
            if command.startswith('routines '):
                return {'ok': True}
            value = json.loads(command.removeprefix('aim-light '))
            action = value['action']
            if action in ('scan', 'target', 'end'):
                state['run'] += 1
                result = {'ok': True, 'run_id': state['run']}
                if action == 'target':
                    result['status'] = aimed
                    state['committed'] = aimed != 'unreachable'
                return result
            if action == 'status':
                return {'ok': True, 'committed': state['committed']}
            return {'ok': True}
        gateway._checked = checked
        gateway.find_object = Mock(side_effect=finds)
        return gateway, commands

    def test_found_waits_commits_lamp_mode_and_closes_explicit_voice_interval(self):
        gateway, commands = self.gateway([{'status': 'found', 'candidates': [{'point': [0, -.3, 0]}]}])
        result = gateway.aim_light({'operation': 'aim_light', 'category': 'book', 'session_id': 'a'*32})
        self.assertEqual((result['status'], result['moved']), ('aimed', True))
        gateway.find_object.assert_called_once_with('book', 'a'*32, deadline=ANY)
        self.assertEqual(commands[0], f'voice {"a"*32} vision_begin')
        self.assertEqual(commands[-1], f'voice {"a"*32} vision_end')
        self.assertIn('routines {"action": "set_mode", "mode": "lamp"}', commands)
        target = next(json.loads(command[10:]) for command in commands if command.startswith('aim-light ') and json.loads(command[10:])['action'] == 'target')
        self.assertEqual(target['class'], 'book')

    def test_no_match_checks_three_authored_stops_then_restores_and_reports_no_match(self):
        gateway, commands = self.gateway([{'status': 'not_found', 'candidates': []}] * 4)
        result = gateway.aim_light({'operation': 'aim_light', 'category': 'book'})
        self.assertEqual(result['status'], 'not_found')
        actions = [json.loads(command[10:]) for command in commands if command.startswith('aim-light ')]
        self.assertEqual([value['index'] for value in actions if value['action'] == 'scan'], [0, 1, 2])
        self.assertEqual(actions[-1]['action'], 'end')
        self.assertFalse(any(value['action'] == 'target' for value in actions))

    def test_ambiguity_and_unreachable_preserve_honest_results_without_choosing_an_object(self):
        for found, status in [({'status': 'ambiguous', 'candidates': [{}, {}]}, 'ambiguous'),
            ({'status': 'found', 'candidates': [{'point': None}]}, 'clamped')]:
            gateway, commands = self.gateway([found])
            result = gateway.aim_light({'operation': 'aim_light', 'category': 'book'})
            self.assertEqual((result['status'], result['moved']), (status, False))
            self.assertFalse(any('"action": "target"' in command for command in commands))

    def test_vision_metadata_cannot_override_operation_success(self):
        gateway, _ = self.gateway([{'status': 'ambiguous', 'candidates': [{}, {}], 'ok': False, 'moved': True}])
        self.assertEqual(gateway.aim_light({'operation': 'aim_light', 'category': 'book'}),
                         {'ok': True, 'moved': False, 'status': 'ambiguous', 'candidates': [{}, {}]})

    def test_failed_lookup_returns_through_owned_end_and_releases_interval(self):
        gateway, commands = self.gateway([GatewayError(503, 'find_timeout', 'Timed out')])
        with self.assertRaises(GatewayError):
            gateway.aim_light({'operation': 'aim_light', 'category': 'book', 'session_id': 'a'*32})
        self.assertTrue(any('"action": "end"' in command for command in commands))
        self.assertEqual(commands[-1], f'voice {"a"*32} vision_end')

    def test_slow_lookup_reserves_recovery_instead_of_starting_another_pass(self):
        gateway, commands = self.gateway([])
        now = [100.]
        def lookup(*args, deadline):
            self.assertGreaterEqual(deadline - now[0], FIND_TIMEOUT_SECONDS)
            now[0] += FIND_TIMEOUT_SECONDS
            return {'status': 'not_found', 'candidates': []}
        gateway.find_object.side_effect = lookup
        with patch('gateway.time.monotonic', side_effect=lambda: now[0]):
            with self.assertRaises(GatewayError) as caught:
                gateway.aim_light({'operation': 'aim_light', 'category': 'book', 'session_id': 'a'*32})
        self.assertEqual(caught.exception.code, 'aim_timeout')
        self.assertEqual(gateway.find_object.call_count, 1)
        self.assertTrue(any('"action": "end"' in command for command in commands))
        self.assertEqual(commands[-1], f'voice {"a"*32} vision_end')
        self.assertLess(now[0], 100 + AIM_TOTAL_SECONDS)

    def test_failed_scan_sends_end_promptly_and_cleanup_cannot_mask_it(self):
        gateway, commands = self.gateway([{'status': 'not_found', 'candidates': []}])
        original = GatewayError(409, 'scan_failed', 'Measured scan failed')
        normal_checked = gateway._checked
        now, ended_at = [100.], []
        def checked(command, *, deadline):
            self.assertLess(now[0], deadline)
            if '"action": "scan"' in command:
                raise original
            if '"action": "end"' in command:
                ended_at.append(now[0])
                raise GatewayError(503, 'end_failed', 'Recovery reply lost')
            if command.endswith('vision_end'):
                raise GatewayError(503, 'voice_end_failed', 'Voice reply lost')
            return normal_checked(command, deadline=deadline)
        gateway._checked = checked
        def sleep(seconds):
            now[0] += seconds
        with patch('gateway.time.monotonic', side_effect=lambda: now[0]), \
             patch('gateway.time.sleep', side_effect=sleep), self.assertLogs('gateway', level='WARNING'):
            with self.assertRaises(GatewayError) as caught:
                gateway.aim_light({'operation': 'aim_light', 'category': 'book', 'session_id': 'a'*32})
        self.assertIs(caught.exception, original)
        self.assertEqual(ended_at, [100.])
        self.assertEqual(now[0], 100 + AIM_RECOVERY_SECONDS - AIM_VOICE_END_SECONDS)

    def test_runtime_request_deadline_is_not_renewed_by_partial_responses(self):
        client = UnixOrionClient(timeout_seconds=2)
        now = [100.]
        socket = Mock()
        socket.__enter__ = Mock(return_value=socket)
        socket.__exit__ = Mock(return_value=False)
        def receive(size):
            now[0] += .6
            return b' '
        socket.recv.side_effect = receive
        with patch('gateway.socket.socket', return_value=socket), \
             patch('gateway.time.monotonic', side_effect=lambda: now[0]):
            with self.assertRaises(GatewayError) as caught:
                client.request('status', deadline=101.)
        self.assertEqual(caught.exception.code, 'runtime_unavailable')
        self.assertEqual(socket.recv.call_count, 2)

    def test_camera_off_and_injected_parameters_are_rejected_before_execution(self):
        gateway, commands = self.gateway([])
        for payload in [{'operation': 'aim_light', 'category': 'book', 'point': [0, -.3, 0]},
                        {'operation': 'aim_light', 'category': 'book', 'session_id': 'bad'}]:
            with self.assertRaises(GatewayError):
                gateway.aim_light(payload)
        gateway.camera_preference_file.write_text('{"camera_enabled":false}')
        with self.assertRaises(GatewayError):
            gateway.aim_light({'operation': 'aim_light', 'category': 'book'})
        self.assertEqual(commands, [])
