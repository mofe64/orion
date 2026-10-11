import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock

from test_gateway import OrionGateway, GatewayError


class AimGatewayTests(unittest.TestCase):
    def gateway(self, finds, aimed='aimed'):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        gateway = OrionGateway(Mock(), hardware='v2', camera_preference_file=Path(temporary.name) / 'camera.json')
        commands = []
        state = {'run': 0, 'session': None, 'interval': None, 'committed': False}
        def checked(command):
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
        gateway.find_object.assert_called_once_with('book', 'a'*32)
        self.assertEqual(commands[0], f'voice {"a"*32} vision_begin')
        self.assertEqual(commands[-1], f'voice {"a"*32} vision_end')
        self.assertIn('routines {"action": "set_mode", "mode": "lamp"}', commands)

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

    def test_failed_lookup_returns_through_owned_end_and_releases_interval(self):
        gateway, commands = self.gateway([GatewayError(503, 'find_timeout', 'Timed out')])
        with self.assertRaises(GatewayError):
            gateway.aim_light({'operation': 'aim_light', 'category': 'book', 'session_id': 'a'*32})
        self.assertTrue(any('"action": "end"' in command for command in commands))
        self.assertEqual(commands[-1], f'voice {"a"*32} vision_end')

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
