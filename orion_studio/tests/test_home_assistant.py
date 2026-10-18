"""Exercise Home Assistant onboarding and the private gateway boundary."""
import base64
from contextlib import redirect_stdout
import hashlib
from http.client import HTTPConnection
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import io
import json
import os
from pathlib import Path
import struct
import sys
import tempfile
import threading
import unittest
from unittest.mock import Mock, patch
from urllib.parse import parse_qs
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from gateway import GatewayHTTPServer, OrionGateway, make_handler, GatewayError
from home_assistant import HomeAssistant, HomeError, private_json


class FakeHA:
    def __init__(self):
        self.done, self.calls, self.services = set(), [], []
        self.exposed = {'light.desk': {'conversation': True}, 'light.second': {'conversation': True},
                        'lock.front': {'conversation': True}, 'alarm_control_panel.home': {'conversation': True}, 'cover.garage': {'conversation': True}}
        self.schema = [{'name': 'password', 'selector': {'text': {'type': 'password'}}, 'required': True}, {'name': 'port', 'type': 'integer', 'default': 80}]
        fake = self
        class Handler(BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'
            def log_message(self, *args):
                pass
            def reply(self, value, status=200):
                raw = json.dumps(value).encode(); self.send_response(status); self.send_header('Content-Type', 'application/json'); self.send_header('Content-Length', str(len(raw))); self.end_headers(); self.wfile.write(raw)
            def body(self):
                raw = self.rfile.read(int(self.headers.get('Content-Length', 0)))
                return parse_qs(raw.decode()) if 'x-www-form-urlencoded' in self.headers.get('Content-Type', '') else json.loads(raw or '{}')
            def do_GET(self):
                if self.path == '/api/websocket':
                    self.websocket(); return
                fake.calls.append(('GET', self.path, None))
                if self.path == '/api/onboarding':
                    self.reply([{'step': step, 'done': step in fake.done} for step in ('user', 'core_config', 'analytics', 'integration')])
                elif self.path == '/api/config': self.reply({'version': '2026.10.2'})
                elif self.path == '/api/config/config_entries/flow_handlers': self.reply(['hue', 'matter'])
                elif self.path.startswith('/api/config/config_entries/flow/'): self.reply({'flow_id': 'flow_1', 'type': 'form', 'step_id': 'user', 'data_schema': fake.schema})
                else: self.reply({'message': 'TEMP-SECRET should not be returned'}, 400)
            def do_POST(self):
                body = self.body(); fake.calls.append(('POST', self.path, body))
                if self.path == '/api/onboarding/users': fake.done.add('user'); self.reply({'auth_code': 'CODE-SECRET'})
                elif self.path == '/auth/token': self.reply({} if body.get('action') == ['revoke'] else {'access_token': 'TEMP-SECRET', 'refresh_token': 'REFRESH-SECRET'})
                elif self.path == '/auth/login_flow': self.reply({'flow_id': 'login'})
                elif self.path == '/auth/login_flow/login': self.reply({'result': 'CODE-SECRET'})
                elif self.path.startswith('/api/onboarding/'):
                    fake.done.add(self.path.rsplit('/', 1)[1]); self.reply({})
                elif self.path.startswith('/api/services/'):
                    fake.services.append((self.path, body)); self.reply([])
                elif self.path.startswith('/api/config/config_entries/flow'):
                    self.reply({'flow_id': 'flow_1', 'type': 'form', 'step_id': 'user', 'data_schema': fake.schema})
                else: self.reply({}, 404)
            def do_DELETE(self):
                fake.calls.append(('DELETE', self.path, None)); self.reply({'message': 'Flow aborted'})
            def send_frame(self, value, opcode=1):
                body = json.dumps(value).encode() if opcode == 1 else value
                size = len(body); prefix = bytes([128 | opcode, size if size < 126 else 126])
                self.wfile.write(prefix + (struct.pack('!H', size) if size >= 126 else b'') + body); self.wfile.flush()
            def receive_frame(self):
                first = self.rfile.read(2)
                if not first: return None
                opcode, length = first[0] & 15, first[1] & 127
                if length == 126: length = struct.unpack('!H', self.rfile.read(2))[0]
                if length == 127: length = struct.unpack('!Q', self.rfile.read(8))[0]
                if not first[1] & 128: raise AssertionError('Client frames must be masked')
                mask = self.rfile.read(4); raw = self.rfile.read(length)
                body = bytes(value ^ mask[i % 4] for i, value in enumerate(raw))
                if opcode == 10: return self.receive_frame()
                return json.loads(body)
            def websocket(self):
                accept = base64.b64encode(hashlib.sha1((self.headers['Sec-WebSocket-Key'] + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
                self.send_response(101); self.send_header('Upgrade', 'websocket'); self.send_header('Connection', 'Upgrade'); self.send_header('Sec-WebSocket-Accept', accept); self.end_headers()
                self.send_frame({'type': 'auth_required'})
                auth = self.receive_frame()
                if auth['access_token'] not in ('TEMP-SECRET', 'LONG-SECRET'):
                    self.send_frame({'type': 'auth_invalid'}); return
                self.send_frame({'type': 'auth_ok'})
                while (message := self.receive_frame()) is not None:
                    kind = message['type']; fake.calls.append(('WS', kind, message))
                    result = None
                    if kind == 'auth/long_lived_access_token': result = 'LONG-SECRET'
                    elif kind == 'config/device_registry/list': result = [{'id': 'desk_device', 'area_id': 'study'}]
                    elif kind == 'config/entity_registry/list': result = [{'entity_id': 'light.desk', 'device_id': 'desk_device'}, {'entity_id': 'light.second', 'area_id': 'lounge'}]
                    elif kind == 'config/area_registry/list': result = [{'area_id': 'study', 'name': 'Study'}, {'area_id': 'lounge', 'name': 'Lounge'}]
                    elif kind == 'homeassistant/expose_entity/list': result = {'exposed_entities': fake.exposed}
                    elif kind == 'homeassistant/expose_entity':
                        for identifier in message['entity_ids']: fake.exposed[identifier] = {'conversation': message['should_expose']}
                    elif kind == 'get_states':
                        result = [{'entity_id': identifier, 'state': 'off', 'attributes': {'friendly_name': name, **attrs}} for identifier, name, attrs in (
                            ('light.desk', 'Desk', {}), ('light.second', 'Desk', {}), ('switch.private', 'Private', {}),
                            ('lock.front', 'Front door', {}), ('alarm_control_panel.home', 'Alarm', {}), ('cover.garage', 'Garage', {'device_class': 'garage'}))]
                    elif kind == 'config_entries/get': result = []
                    elif kind == 'config_entries/flow/progress': result = [{'flow_id': 'flow_1', 'context': {'source': 'zeroconf'}}, {'flow_id': 'other', 'context': {'source': 'user'}}]
                    self.send_frame(b'ping', opcode=9)
                    self.send_frame({'id': message['id'], 'type': 'result', 'success': True, 'result': result})
        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever); self.thread.start()
    def close(self):
        self.server.shutdown(); self.server.server_close(); self.thread.join()


class HomeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.fake = FakeHA(); self.addCleanup(self.fake.close)
        self.home = HomeAssistant(self.root / 'config/home-assistant.json', self.root / 'data', port=self.fake.server.server_port)
        self.owner = {'name': 'Owner', 'username': 'owner', 'password': 'PASSWORD-SECRET'}
    def connect(self):
        private_json(self.home.token_file, {'access_token': 'LONG-SECRET'})
    def test_onboarding_secure_token_timezone_location_analytics_and_resume(self):
        self.home.data.mkdir(); (self.home.data / 'weather.json').write_text(json.dumps({'location': {'name': 'London', 'latitude': 51.5, 'longitude': -0.1}}))
        output = io.StringIO()
        with redirect_stdout(output), patch('home_assistant.pi_timezone', return_value='Europe/London'):
            result = self.home.onboarding(self.owner)
        self.assertEqual(result, {'onboarded': True, 'connected': True})
        self.assertEqual(self.home.token_file.stat().st_mode & 0o777, 0o600)
        self.assertEqual(json.loads(self.home.token_file.read_text()), {'access_token': 'LONG-SECRET'})
        self.assertEqual(self.fake.done, {'user', 'core_config', 'analytics', 'integration'})
        config = next(body for method, path, body in self.fake.calls if path == 'config/core/update')
        self.assertEqual((config['time_zone'], config['latitude'], config['longitude']), ('Europe/London', 51.5, -0.1))
        analytics = next(body for _, path, body in self.fake.calls if path == 'analytics/preferences')
        self.assertTrue(all(value is False for value in analytics['preferences'].values()))
        self.assertTrue(self.home.status()['connected'])
        for secret in ('LONG-SECRET', 'TEMP-SECRET', 'PASSWORD-SECRET', 'REFRESH-SECRET'):
            self.assertNotIn(secret, json.dumps(result) + output.getvalue())
        before = len([path for _, path, _ in self.fake.calls if path == '/api/onboarding/users'])
        self.home.onboarding(self.owner)
        self.assertEqual(len([path for _, path, _ in self.fake.calls if path == '/api/onboarding/users']), before)
    def test_interrupted_onboarding_refreshes_private_session_and_finishes(self):
        self.fake.done.add('user')
        private_json(self.home.token_file, {'access_token': 'expired', 'refresh_token': 'REFRESH-SECRET'})
        self.home.onboarding(self.owner)
        requests = [body for method, path, body in self.fake.calls if path == '/auth/token']
        self.assertEqual(requests[0]['grant_type'], ['refresh_token'])
        self.assertEqual(self.home.credentials(), {'access_token': 'LONG-SECRET'})

    def test_owner_creation_failure_recovers_through_login(self):
        self.fake.done.add('user')
        self.home.onboarding(self.owner)
        self.assertTrue(any(path == '/auth/login_flow/login' for _, path, _ in self.fake.calls))
    def test_flow_passthrough_and_discovery(self):
        self.connect()
        flow = self.home.flow('POST', payload={'handler': 'hue'})
        self.assertEqual(flow['data_schema'], self.fake.schema)
        self.assertEqual(self.home.flow('POST', 'flow_1', {'password': 'integration-secret'})['data_schema'], self.fake.schema)
        self.home.flow('DELETE', 'flow_1')
        self.assertEqual(len(self.home.discovered()), 1)
        self.assertFalse(self.home.discovered()[0]['ignored'])
        self.assertFalse(any(method == 'DELETE' for method, _, _ in self.fake.calls))
        self.assertEqual(self.home.flow('GET', 'flow_1')['flow_id'], 'flow_1')
        with self.assertRaises(HomeError): self.home.flow('GET', '../auth')
    def test_voice_allowlist_name_ambiguity_refused_domains_and_single_target(self):
        self.connect()
        listed = self.home.voice({'action': 'list'})['devices']
        self.assertNotIn('switch.private', [entry['entity_id'] for entry in listed])
        with self.assertRaisesRegex(HomeError, 'Which device'): self.home.voice({'action': 'control', 'name': 'Desk', 'command': 'on'})
        self.home.voice({'action': 'control', 'name': 'Desk', 'area': 'Study', 'command': 'on', 'brightness': 50})
        self.assertEqual(self.fake.services[-1], ('/api/services/light/turn_on', {'entity_id': 'light.desk', 'brightness_pct': 50}))
        self.home.expose({'entity_id': 'light.desk', 'exposed': False})
        for action in ('state', 'control'):
            with self.assertRaises(HomeError): self.home.voice({'action': action, 'entity': 'light.desk', 'command': 'on'})
        for identifier in ('lock.front', 'alarm_control_panel.home', 'cover.garage'):
            with self.assertRaisesRegex(HomeError, 'cannot control locks'): self.home.voice({'action': 'control', 'entity': identifier, 'command': 'on'})
        with self.assertRaisesRegex(HomeError, 'cannot control locks'):
            self.home.control({'entity_id': 'cover.drive', 'domain': 'cover', 'attributes': {'device_class': 'gate'}}, {'command': 'open'})
        for domain in ('script', 'scene'):
            with self.assertRaisesRegex(HomeError, 'that kind'):
                self.home.control({'entity_id': f'{domain}.leave_home', 'domain': domain, 'attributes': {}}, {'command': 'on'})
        with self.assertRaises(HomeError): self.home.voice({'action': 'control', 'entity': 'light.second', 'command': 'on', 'brightness': float('nan')})
        self.assertEqual(len(self.fake.services), 1)
    def test_gateway_onboarding_never_returns_or_logs_secrets(self):
        gateway = OrionGateway(Mock()); gateway.home = self.home
        server = GatewayHTTPServer(('127.0.0.1', 0), make_handler(gateway, 'studio-secret', []))
        thread = threading.Thread(target=server.serve_forever); thread.start()
        output = io.StringIO()
        try:
            with redirect_stdout(output):
                connection = HTTPConnection(*server.server_address)
                connection.request('POST', '/api/v2/home/onboarding', json.dumps(self.owner), {'Authorization': 'Bearer studio-secret', 'Content-Type': 'application/json'})
                response = connection.getresponse(); raw = response.read().decode(); connection.close()
                self.assertEqual(response.status, 200)
            for secret in ('PASSWORD-SECRET', 'CODE-SECRET', 'TEMP-SECRET', 'REFRESH-SECRET', 'LONG-SECRET'):
                self.assertNotIn(secret, raw + output.getvalue())
            self.assertEqual(json.loads(raw), {'onboarded': True, 'connected': True})
        finally:
            server.shutdown(); server.server_close(); thread.join()

    def test_gateway_authenticated_routes_token_privacy_and_voice_policy(self):
        self.connect(); gateway = OrionGateway(Mock()); gateway.home = self.home
        server = GatewayHTTPServer(('127.0.0.1', 0), make_handler(gateway, 'studio-secret', []))
        thread = threading.Thread(target=server.serve_forever); thread.start()
        output = io.StringIO()
        try:
            with redirect_stdout(output):
                for route in ('status', 'devices', 'discovered', 'integrations', 'access'):
                    for token, expected in (('', 401), ('studio-secret', 200)):
                        connection = HTTPConnection(*server.server_address)
                        connection.request('GET', '/api/v2/home/' + route, headers={'Authorization': 'Bearer ' + token})
                        response = connection.getresponse(); raw = response.read().decode(); connection.close()
                        self.assertEqual(response.status, expected)
                        self.assertNotIn('LONG-SECRET', raw)
                connection = HTTPConnection(*server.server_address)
                connection.request('POST', '/api/v2/operations', json.dumps({'operation': 'home', 'request': {'action': 'control', 'entity': 'switch.private', 'command': 'on'}}), {'Authorization': 'Bearer studio-secret', 'Content-Type': 'application/json'})
                response = connection.getresponse(); self.assertEqual(response.status, 409); response.read(); connection.close()
            self.assertNotIn('LONG-SECRET', output.getvalue())
            with self.assertRaises(GatewayError): gateway.home_request('GET', 'not-allowed')
            self.assertEqual(gateway._home_public({'nested': ['LONG-SECRET']}), {'nested': ['[private]']})
            with self.assertRaises(HomeError) as error: self.home.rest('/rejected')
            self.assertNotIn('TEMP-SECRET', str(error.exception))
        finally:
            server.shutdown(); server.server_close(); thread.join()


class HouseholdTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.home = HomeAssistant(data=Path(self.temp.name))
        self.fixture = json.loads((Path(__file__).parent / 'fixtures/home_assistant.json').read_text())
        self.peer = Mock()
        def command(kind, **fields):
            return {'config/device_registry/list': self.fixture['devices'], 'config/entity_registry/list': self.fixture['entities'],
                    'config/area_registry/list': self.fixture['areas'], 'get_states': self.fixture['states'],
                    'homeassistant/expose_entity/list': {'exposed_entities': self.fixture['exposed']},
                    'config_entries/flow/progress': self.fixture['discovered'], 'config_entries/get': self.fixture.get('entries', [])}.get(kind)
        self.peer.command.side_effect = command
        self.home.session = Mock(return_value=self.peer)
        self.home.rest = Mock(return_value={})

    def test_real_household_filtering_grouping_and_voice_use_same_inventory(self):
        inventory = self.home.devices()
        identifiers = {entity['entity_id'] for area in inventory['areas'] for entity in area['entities']}
        expected = {'light.reading_lamp', 'sensor.lamp_power', 'media_player.tv', 'climate.living', 'cover.blind',
                    'fan.bedside', 'switch.coffee', 'sensor.bedroom_temperature', 'sensor.bedroom_battery', 'lock.front_door', 'cover.garage'}
        self.assertEqual(identifiers, expected)
        self.assertEqual(len(inventory['devices']), 9)
        lamp = next(device for device in inventory['devices'] if device['id'] == 'lamp')
        self.assertEqual(len(lamp['entities']), 2)
        self.assertEqual(next(device for device in inventory['devices'] if device['id'] == 'plug')['area'], 'No room yet')
        for state in self.fixture['states']:
            self.fixture['exposed'][state['entity_id']] = {'conversation': True}
        self.assertEqual({entry['entity_id'] for entry in self.home.voice({'action': 'list'})['devices']}, expected)
        with self.assertRaises(HomeError): self.home.voice({'action': 'control', 'entity': 'switch.lamp_config', 'command': 'on'})
        self.home.rest.assert_not_called()

    def test_device_access_only_updates_safe_controls(self):
        self.fixture['entities'].append({'entity_id': 'switch.lamp_socket', 'device_id': 'lamp', 'platform': 'hue'})
        self.fixture['states'].append({'entity_id': 'switch.lamp_socket', 'state': 'off', 'attributes': {}})
        self.home.expose({'device_id': 'lamp', 'exposed': True})
        self.peer.command.assert_called_with('homeassistant/expose_entity', assistants=['conversation'], entity_ids=['light.reading_lamp', 'switch.lamp_socket'], should_expose=True)
        for identifier in ('door', 'garage', 'weather', 'missing'):
            with self.assertRaises(HomeError): self.home.expose({'device_id': identifier, 'exposed': True})
        with self.assertRaises(HomeError): self.home.expose({'entity_id': 'switch.lamp_config', 'exposed': True})

    def test_room_assignment_updates_device_and_clears_entity_overrides(self):
        self.home.room({'device_id': 'lamp', 'area_id': 'bedroom'})
        self.assertTrue(any(call.args[0] == 'config/device_registry/update' and call.kwargs == {'device_id': 'lamp', 'area_id': 'bedroom'} for call in self.peer.command.call_args_list))
        self.peer.command.assert_called_with('config/entity_registry/update', entity_id='sensor.lamp_power', area_id=None)
        self.home.room({'device_id': 'lamp', 'area_id': None})
        with self.assertRaises(HomeError): self.home.room({'device_id': 'lamp', 'area_id': 'missing'})
        with self.assertRaises(HomeError): self.home.room({'device_id': 'missing', 'area_id': None})

    def test_entities_on_one_device_share_the_same_room_and_service_devices_are_hidden(self):
        for device in self.fixture['devices']:
            if device['id'] == 'lamp': device['area_id'] = None
            if device['id'] == 'weather': device['entry_type'] = 'service'
        for entity in self.fixture['entities']:
            if entity['entity_id'] == 'light.reading_lamp': entity['area_id'] = 'living'
            if entity['entity_id'] == 'sensor.lamp_power': entity['area_id'] = 'bedroom'
        result = self.home.devices()
        lamp = next(device for device in result['devices'] if device['id'] == 'lamp')
        self.assertEqual({entity['area'] for entity in lamp['entities']}, {lamp['area']})
        self.assertNotIn('weather', {device['id'] for device in result['devices']})

    def test_manual_control_does_not_require_voice_permission_and_validates_settings(self):
        entities = [entity for area in self.home.devices()['areas'] for entity in area['entities']]
        television = next(entity for entity in entities if entity['entity_id'] == 'media_player.tv')
        self.assertFalse(television['exposed'])
        self.home.control(television, {'command': 'volume', 'volume': 35})
        self.home.rest.assert_called_with('/api/services/media_player/volume_set', 'POST', {'entity_id': 'media_player.tv', 'volume_level': .35})
        for value in (-1, 101, True, float('nan'), '30'):
            with self.assertRaises(HomeError): self.home.control(television, {'command': 'volume', 'volume': value})
        with self.assertRaises(HomeError): self.home.control(television, {'command': 'volume'})
        for identifier in ('cover.garage', 'lock.front_door'):
            with self.assertRaises(HomeError): self.home.control(next(entity for entity in entities if entity['entity_id'] == identifier), {'command': 'open'})

    def test_manual_controls_route_through_the_gateway_for_each_supported_type(self):
        gateway = OrionGateway(Mock()); gateway.home = self.home
        for identifier, command, extra, service in (
            ('light.reading_lamp', 'on', {'brightness': 60}, 'light/turn_on'),
            ('switch.coffee', 'on', {}, 'switch/turn_on'), ('fan.bedside', 'off', {}, 'fan/turn_off'),
            ('climate.living', 'temperature', {'temperature': 22}, 'climate/set_temperature'),
            ('cover.blind', 'open', {}, 'cover/open_cover'), ('cover.blind', 'close', {}, 'cover/close_cover'),
            ('cover.blind', 'stop', {}, 'cover/stop_cover'), ('media_player.tv', 'play', {}, 'media_player/media_play'),
            ('media_player.tv', 'pause', {}, 'media_player/media_pause'), ('media_player.tv', 'off', {}, 'media_player/turn_off')):
            with self.subTest(identifier=identifier, command=command):
                gateway.home_request('POST', 'control', {'entity_id': identifier, 'command': command, **extra})
                self.assertEqual(self.home.rest.call_args.args[0], '/api/services/' + service)
        gateway.home_request('POST', 'room', {'device_id': 'lamp', 'area_id': 'bedroom'})
        with self.assertRaises(GatewayError): gateway.home_request('POST', 'control', {'entity_id': 'switch.lamp_config', 'command': 'on'})

    def test_cancel_preserves_discovery_across_gateway_restart_and_ignored_can_restart(self):
        private_json(self.home.data / 'home-hidden.json', ['tv_setup'])
        self.home.flow('DELETE', 'tv_setup')
        self.home.rest.assert_not_called()
        self.assertFalse(self.home.discovered()[0]['ignored'])
        again = HomeAssistant(data=Path(self.temp.name)); again.session = self.home.session
        self.assertFalse(again.discovered()[0]['ignored'])
        self.home.set_discovery({'flow_id':'tv_setup','ignored':True})
        self.assertTrue(again.discovered()[0]['ignored'])
        self.home.flow('DELETE','tv_setup')
        self.assertTrue(again.discovered()[0]['ignored'])
        again.set_discovery({'flow_id':'tv_setup','ignored':False})
        self.assertFalse(self.home.discovered()[0]['ignored'])
        with self.assertRaises(HomeError): self.home.set_discovery({'flow_id':'missing','ignored':True})
        with self.assertRaises(HomeError): self.home.set_discovery({'flow_id':'tv_setup','ignored':'false'})
        self.home.rest.return_value = {'type': 'form', 'flow_id': 'tv_setup'}
        self.assertEqual(self.home.flow('GET', 'tv_setup')['flow_id'], 'tv_setup')
        self.fixture['entries'] = [{'source': 'ignore', 'entry_id': 'ignored_tv', 'domain': 'samsungtv', 'title': 'Bedroom TV'}]
        self.assertEqual(self.home.discovered()[-1]['ignored_entry_id'], 'ignored_tv')
        self.home.flow('POST', payload={'handler': 'samsungtv', 'ignored_entry_id': 'ignored_tv'})
        self.home.rest.assert_any_call('/api/config/config_entries/entry/ignored_tv', 'DELETE')
        self.home.rest.assert_called_with('/api/config/config_entries/flow', 'POST', {'handler': 'samsungtv'})
        with self.assertRaises(HomeError): self.home.flow('POST', payload={'handler': 'hue', 'ignored_entry_id': 'ignored_tv'})

    def test_completed_setup_does_not_return_device_credentials(self):
        self.home.rest.return_value = {'type': 'create_entry', 'data': {'password': 'DEVICE-SECRET'}, 'result': {'entry_id': 'lamp'}}
        result = self.home.flow('POST', payload={'handler': 'hue'})
        self.assertNotIn('DEVICE-SECRET', json.dumps(result))
        self.assertEqual(result['result']['entry_id'], 'lamp')
