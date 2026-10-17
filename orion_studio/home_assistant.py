"""Pi-local Home Assistant transport and household access policy."""
import base64
import hashlib
import http.client
import json
import math
import os
from pathlib import Path
import re
import secrets
import socket
import struct
import tempfile
import threading
from urllib.parse import quote, urlencode

LIMIT = 4 * 1024 * 1024
CLIENT_ID = 'http://127.0.0.1:7447/'
DISCOVERY = {'zeroconf', 'ssdp', 'dhcp', 'bluetooth', 'homekit', 'homekit_controller'}
DOMAINS = {'light', 'switch', 'fan', 'climate', 'cover', 'media_player', 'scene', 'script'}


class HomeError(Exception):
    def __init__(self, message='Home Assistant could not complete this request.', status=502):
        super().__init__(message)
        self.status = status


def private_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode='w', dir=path.parent, prefix='.home-', delete=False) as output:
            temporary = Path(output.name)
            os.fchmod(output.fileno(), 0o600)
            json.dump(value, output); output.flush(); os.fsync(output.fileno())
        temporary.replace(path)
    finally:
        if temporary:
            temporary.unlink(missing_ok=True)


class WebSocket:
    """One authenticated command session with bounded RFC 6455 frames."""
    def __init__(self, host, port, token):
        self.socket = socket.create_connection((host, port), timeout=4)
        self.stream = self.socket.makefile('rb')
        self.identifier = 0
        try:
            key = base64.b64encode(secrets.token_bytes(16)).decode()
            self.socket.sendall((f'GET /api/websocket HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n').encode())
            status = self.stream.readline(4096)
            headers, size = {}, 0
            while True:
                line = self.stream.readline(4096); size += len(line)
                if size > 16384 or not line:
                    raise HomeError()
                if line == b'\r\n':
                    break
                name, value = line.decode().split(':', 1); headers[name.lower()] = value.strip()
            expected = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
            if not status.startswith(b'HTTP/1.1 101 ') or headers.get('sec-websocket-accept') != expected:
                raise HomeError()
            if self.receive().get('type') != 'auth_required':
                raise HomeError()
            self.send({'type': 'auth', 'access_token': token})
            if self.receive().get('type') != 'auth_ok':
                raise HomeError('Home Assistant credentials need attention.', 401)
        except Exception:
            self.close()
            raise

    def close(self):
        self.stream.close(); self.socket.close()

    def _exact(self, size):
        value = self.stream.read(size)
        if len(value) != size:
            raise HomeError('Home Assistant closed the connection.')
        return value

    def _frame(self, body, opcode=1):
        if len(body) > LIMIT:
            raise HomeError('Home Assistant request is too large.', 413)
        mask = secrets.token_bytes(4)
        size = len(body)
        prefix = bytes([0x80 | opcode, 0x80 | (size if size < 126 else 126 if size < 65536 else 127)])
        if size >= 126:
            prefix += struct.pack('!H' if size < 65536 else '!Q', size)
        self.socket.sendall(prefix + mask + bytes(value ^ mask[i % 4] for i, value in enumerate(body)))

    def send(self, value):
        self._frame(json.dumps(value, allow_nan=False).encode())

    def receive(self):
        chunks, size, fragmented = [], 0, False
        for _ in range(256):
            first, second = self._exact(2)
            opcode, final = first & 15, bool(first & 128)
            if first & 0x70 or second & 128:
                raise HomeError()
            length = second & 127
            if length == 126:
                length = struct.unpack('!H', self._exact(2))[0]
            elif length == 127:
                length = struct.unpack('!Q', self._exact(8))[0]
            if length > LIMIT or size + length > LIMIT:
                raise HomeError('Home Assistant response is too large.')
            body = self._exact(length)
            if opcode == 8:
                raise HomeError('Home Assistant closed the connection.')
            if opcode in (9, 10):
                if not final or length > 125:
                    raise HomeError()
                if opcode == 9:
                    self._frame(body, 10)
                continue
            if opcode not in (0, 1) or (opcode == 0) != fragmented:
                raise HomeError()
            chunks.append(body); size += length; fragmented = not final
            if final:
                return json.loads(b''.join(chunks))
        raise HomeError()

    def command(self, kind, **fields):
        self.identifier += 1
        self.send({'id': self.identifier, 'type': kind, **fields})
        for _ in range(100):
            value = self.receive()
            if value.get('id') == self.identifier and value.get('type') == 'result':
                if value.get('success') is not True:
                    raise HomeError('Home Assistant refused this request. Check its configuration.', 400)
                return value.get('result')
        raise HomeError()


class HomeAssistant:
    def __init__(self, token_file=None, data=None, host='127.0.0.1', port=8123):
        self.token_file = token_file or Path.home() / '.config/orion/home-assistant.json'
        self.data = data or Path.home() / '.local/share/orion'
        self.host, self.port = host, port
        self.setup_lock = threading.Lock()

    def credentials(self):
        try:
            return json.loads(self.token_file.read_text())
        except (OSError, ValueError):
            return {}

    def token(self):
        value = self.credentials().get('access_token')
        if not isinstance(value, str) or not value:
            raise HomeError('Set up Home in Studio first.', 409)
        return value

    def rest(self, path, method='GET', body=None, *, auth=True, token=None, form=False):
        connection = http.client.HTTPConnection(self.host, self.port, timeout=4)
        try:
            headers = {'Content-Type': 'application/x-www-form-urlencoded' if form else 'application/json'}
            if auth:
                headers['Authorization'] = 'Bearer ' + (token or self.token())
            encoded = (urlencode(body) if form else json.dumps(body, allow_nan=False)) if body is not None else None
            connection.request(method, path, encoded, headers)
            response = connection.getresponse()
            raw = response.read(LIMIT + 1)
            if len(raw) > LIMIT:
                raise HomeError('Home Assistant response is too large.')
            if response.status >= 300:
                raise HomeError('Home Assistant refused this request. Check its configuration.', response.status if response.status in (400, 401, 403, 404, 409) else 502)
            return json.loads(raw) if raw else {}
        except (OSError, ValueError, http.client.HTTPException):
            raise HomeError('Home Assistant is unavailable. Wait for it to start.', 503) from None
        finally:
            connection.close()

    def session(self, token=None):
        try:
            return WebSocket(self.host, self.port, token or self.token())
        except (OSError, ValueError, KeyError):
            raise HomeError('Home Assistant is unavailable. Wait for it to start.', 503) from None

    def ws(self, kind, **fields):
        peer = self.session()
        try:
            return peer.command(kind, **fields)
        except (OSError, ValueError, KeyError):
            raise HomeError() from None
        finally:
            peer.close()

    def status(self):
        try:
            installed = json.loads((self.data / 'home-assistant-install.json').read_text())
        except (OSError, ValueError):
            installed = {}
        result = {'installed': bool(installed), 'running': False, 'version': installed.get('version'),
                  'onboarded': False, 'connected': False}
        try:
            steps = self.rest('/api/onboarding', auth=False)
            result['running'] = True
            result['installed'] = True
            result['onboarded'] = isinstance(steps, list) and all(step.get('done') for step in steps)
        except HomeError as error:
            if error.status == 404:
                result.update(installed=True, running=True, onboarded=True)
        if result['running'] and self.credentials().get('access_token'):
            try:
                config = self.rest('/api/config')
                result.update(connected=True, version=config.get('version', result['version']))
            except HomeError:
                pass
        return result

    def onboarding(self, payload):
        if not isinstance(payload, dict) or set(payload) != {'name', 'username', 'password'} or any(not isinstance(value, str) or not value.strip() or len(value) > 256 for value in payload.values()):
            raise HomeError('Enter a name, username and password.', 400)
        with self.setup_lock:
            credentials = self.credentials()
            try:
                steps = self.rest('/api/onboarding', auth=False)
            except HomeError as error:
                if error.status != 404:
                    raise
                steps = []
            done = {step['step'] for step in steps if step['done']}
            if credentials.get('refresh_token'):
                renewed = self.rest('/auth/token', 'POST', {'grant_type': 'refresh_token', 'refresh_token': credentials['refresh_token'], 'client_id': CLIENT_ID}, auth=False, form=True)
                credentials['access_token'] = renewed['access_token']
                private_json(self.token_file, credentials)
            elif credentials.get('access_token'):
                try:
                    self.rest('/api/config', token=credentials['access_token'])
                except HomeError as error:
                    if error.status not in (401, 403):
                        raise
                    credentials = {}
            if not credentials.get('access_token'):
                if 'user' not in done and steps:
                    auth = self.rest('/api/onboarding/users', 'POST', {**payload, 'client_id': CLIENT_ID, 'language': 'en'}, auth=False)
                else:
                    # Resume after owner creation, or connect an existing installation.
                    flow = self.rest('/auth/login_flow', 'POST', {'client_id': CLIENT_ID, 'handler': ['homeassistant', None], 'redirect_uri': CLIENT_ID}, auth=False)
                    auth = self.rest('/auth/login_flow/' + quote(flow['flow_id'], safe=''), 'POST', {'client_id': CLIENT_ID, 'username': payload['username'], 'password': payload['password']}, auth=False)
                    auth = {'auth_code': auth.get('result')}
                credentials = self.rest('/auth/token', 'POST', {'grant_type': 'authorization_code', 'code': auth['auth_code'], 'client_id': CLIENT_ID}, auth=False, form=True)
                credentials = {key: credentials[key] for key in ('access_token', 'refresh_token')}
                private_json(self.token_file, credentials)
            peer = self.session(credentials['access_token'])
            try:
                if steps and 'core_config' not in done:
                    config = {'time_zone': pi_timezone()}
                    try:
                        location = json.loads((self.data / 'weather.json').read_text()).get('location')
                        if location:
                            lat, lon = float(location['latitude']), float(location['longitude'])
                            if math.isfinite(lat) and math.isfinite(lon) and -90 <= lat <= 90 and -180 <= lon <= 180:
                                config.update(latitude=lat, longitude=lon, location_name=location['name'])
                    except (OSError, ValueError, KeyError, TypeError):
                        pass
                    peer.command('config/core/update', **config)
                    self.rest('/api/onboarding/core_config', 'POST', {}, token=credentials['access_token'])
                if steps and 'analytics' not in done:
                    peer.command('analytics/preferences', preferences={key: False for key in ('base', 'diagnostics', 'statistics', 'usage', 'snapshots')})
                    self.rest('/api/onboarding/analytics', 'POST', {}, token=credentials['access_token'])
                if steps and 'integration' not in done:
                    self.rest('/api/onboarding/integration', 'POST', {'client_id': CLIENT_ID, 'redirect_uri': CLIENT_ID}, token=credentials['access_token'])
                if 'refresh_token' in credentials:
                    peer.command('homeassistant/expose_new_entities/set', assistant='conversation', expose_new=False)
                    token = peer.command('auth/long_lived_access_token', client_name='Orion ' + secrets.token_hex(4), lifespan=3650)
                    if not isinstance(token, str) or not token:
                        raise HomeError()
                    private_json(self.token_file, {'access_token': token})
                    self.rest('/auth/token', 'POST', {'action': 'revoke', 'token': credentials['refresh_token']}, auth=False, form=True)
            finally:
                peer.close()
            return {'onboarded': True, 'connected': True}

    def devices(self):
        peer = self.session()
        try:
            devices = peer.command('config/device_registry/list')
            entities = peer.command('config/entity_registry/list')
            areas = peer.command('config/area_registry/list')
            exposed = peer.command('homeassistant/expose_entity/list')['exposed_entities']
            states = peer.command('get_states')
        finally:
            peer.close()
        by_device = {device['id']: device for device in devices}
        by_entity = {entity['entity_id']: entity for entity in entities}
        by_area = {area['area_id']: area['name'] for area in areas}
        rooms = {}
        for state in states:
            identifier = state['entity_id']
            registry = by_entity.get(identifier, {})
            device = by_device.get(registry.get('device_id'), {})
            area_id = registry.get('area_id') or device.get('area_id')
            room = by_area.get(area_id, 'Unassigned')
            attributes = state.get('attributes', {})
            entity = {'entity_id': identifier, 'name': registry.get('name') or attributes.get('friendly_name') or registry.get('original_name') or identifier,
                      'area': room, 'area_id': area_id, 'device_id': device.get('id'), 'domain': identifier.split('.')[0],
                      'state': state['state'], 'attributes': attributes, 'exposed': exposed.get(identifier, {}).get('conversation') is True}
            rooms.setdefault(room, []).append(entity)
        return {'areas': [{'name': room, 'entities': items} for room, items in sorted(rooms.items())],
                'devices': devices, 'entities': entities, 'area_registry': areas}

    def expose(self, payload):
        if not isinstance(payload, dict) or set(payload) != {'entity_id', 'exposed'} or type(payload['exposed']) is not bool:
            raise HomeError('Choose an entity and an access setting.', 400)
        identifier = entity_id(payload['entity_id'])
        self.ws('homeassistant/expose_entity', assistants=['conversation'], entity_ids=[identifier], should_expose=payload['exposed'])
        return {'entity_id': identifier, 'exposed': payload['exposed']}

    def discovered(self):
        return [flow for flow in self.ws('config_entries/flow/progress') if flow.get('context', {}).get('source') in DISCOVERY]

    def flow(self, method, identifier=None, payload=None):
        path = '/api/config/config_entries/flow'
        if identifier:
            if not re.fullmatch(r'[a-zA-Z0-9_-]{1,128}', identifier):
                raise HomeError('Invalid Home Assistant flow.', 400)
            path += '/' + identifier
        elif method != 'POST' or not isinstance(payload, dict) or set(payload) != {'handler'} or not isinstance(payload['handler'], str):
            raise HomeError('Choose an integration.', 400)
        return self.rest(path, method, payload)

    def voice(self, payload):
        if not isinstance(payload, dict) or payload.get('action') not in ('list', 'state', 'control'):
            raise HomeError('Invalid home request.', 400)
        entries = [entity for area in self.devices()['areas'] for entity in area['entities'] if entity['exposed']]
        def summary(entity):
            return {key: entity[key] for key in ('entity_id', 'name', 'area', 'domain', 'state')}
        if payload['action'] == 'list':
            return {'ok': True, 'devices': [summary(entity) for entity in entries]}
        target = payload.get('entity') or payload.get('name')
        if not isinstance(target, str) or not target.strip():
            raise HomeError('Tell me which home device you mean.', 400)
        area = payload.get('area')
        matches = [entity for entity in entries if (entity['entity_id'].casefold() == target.casefold() or entity['name'].casefold() == target.casefold()) and (not area or entity['area'].casefold() == str(area).casefold())]
        if len(matches) != 1:
            raise HomeError('Which device do you mean? Please give its room.' if matches else 'That device is not available to Orion. Enable Orion can control in Home.', 409)
        entity = matches[0]
        if payload['action'] == 'state':
            return {'ok': True, 'device': summary(entity)}
        return self.control(entity, payload)

    def control(self, entity, payload):
        domain = entity['domain']
        if domain in ('lock', 'alarm_control_panel') or domain == 'cover' and entity.get('attributes', {}).get('device_class') in ('garage', 'garage_door'):
            raise HomeError('I cannot control locks, alarms or garage doors. Please use Home Assistant.', 403)
        if domain not in DOMAINS:
            raise HomeError('I cannot control that kind of home device.', 400)
        action = payload.get('service') or payload.get('command')
        actions = {'light': {'on': 'turn_on', 'off': 'turn_off', 'toggle': 'toggle'},
                   'switch': {'on': 'turn_on', 'off': 'turn_off', 'toggle': 'toggle'},
                   'fan': {'on': 'turn_on', 'off': 'turn_off', 'toggle': 'toggle'},
                   'climate': {'on': 'turn_on', 'off': 'turn_off', 'temperature': 'set_temperature'},
                   'cover': {'open': 'open_cover', 'close': 'close_cover', 'stop': 'stop_cover', 'position': 'set_cover_position'},
                   'media_player': {'on': 'turn_on', 'off': 'turn_off', 'play': 'media_play', 'pause': 'media_pause', 'stop': 'media_stop'},
                   'scene': {'on': 'turn_on', 'activate': 'turn_on'}, 'script': {'on': 'turn_on', 'run': 'turn_on'}}
        service = actions[domain].get(action)
        if service is None:
            raise HomeError('That action is not supported for this home device.', 400)
        data = {'entity_id': entity_id(entity['entity_id'])}
        temperature_setting = ('temperature', 'color_temp_kelvin', 1000, 40000, 'light') if domain == 'light' else ('temperature', 'temperature', -50, 100, 'climate')
        for field, destination, minimum, maximum, allowed in (
            ('brightness', 'brightness_pct', 0, 100, 'light'), temperature_setting,
            ('position', 'position', 0, 100, 'cover')):
            value = payload.get(field)
            if value is not None:
                if domain != allowed or isinstance(value, bool) or not isinstance(value, (float, int)) or not math.isfinite(value) or not minimum <= value <= maximum:
                    raise HomeError('That device setting is invalid.', 400)
                data[destination] = value
        colour = payload.get('colour')
        if colour is not None:
            if domain != 'light' or not isinstance(colour, str) or not re.fullmatch(r'[a-zA-Z ]{1,40}', colour):
                raise HomeError('Use a colour name for the light.', 400)
            data['color_name'] = colour
        self.rest('/api/services/' + domain + '/' + service, 'POST', data)
        return {'ok': True, 'entity_id': entity['entity_id'], 'service': service}


def entity_id(value):
    if not isinstance(value, str) or not re.fullmatch(r'[a-z_]+\.[a-z0-9_]+', value):
        raise HomeError('Choose one home entity.', 400)
    return value


def pi_timezone():
    from zoneinfo import ZoneInfo, ZoneInfoNotFoundError
    values = [os.environ.get('TZ', '')]
    try:
        values.append(Path('/etc/timezone').read_text().strip())
    except OSError:
        pass
    try:
        values.append(str(Path('/etc/localtime').resolve()).split('/zoneinfo/', 1)[1])
    except (OSError, IndexError):
        pass
    for value in values:
        try:
            ZoneInfo(value)
            return value
        except (ValueError, ZoneInfoNotFoundError):
            continue
    return 'UTC'
