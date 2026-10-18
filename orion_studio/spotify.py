"""Lamp-owned Spotify PKCE credentials and Web API playback."""
from __future__ import annotations

import base64
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import secrets
import tempfile
import threading
import time
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode
from urllib.request import Request, build_opener, HTTPRedirectHandler

REDIRECT_URI = 'http://127.0.0.1:49687/callback'
DASHBOARD = 'https://developer.spotify.com/dashboard'
SCOPES = 'streaming user-read-private user-read-playback-state user-modify-playback-state'


class SpotifyError(Exception):
    def __init__(self, message, status=400):
        super().__init__(message)
        self.status = status


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


def transport(method, url, headers, body):
    try:
        with build_opener(NoRedirect).open(Request(url, data=body, headers=headers, method=method), timeout=5) as response:
            data = response.read(1024 * 1024)
            return response.status, json.loads(data) if data else {}, dict(response.headers)
    except HTTPError as error:
        try: data = json.loads(error.read(16_384))
        except (ValueError, OSError): data = {}
        return error.code, data, dict(error.headers)
    except (OSError, URLError, ValueError):
        raise SpotifyError("Orion couldn't reach Spotify. Check the lamp's internet connection and try again.", 503) from None


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix='.spotify-', dir=path.parent)
    try:
        with os.fdopen(descriptor, 'w') as output:
            json.dump(value, output); output.write('\n'); output.flush(); os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary): os.unlink(temporary)


def friendly(status, body, *, token=False):
    error = body.get('error', {})
    reason = str(error.get('reason', '') if isinstance(error, dict) else error)
    message = str(error.get('message', '') if isinstance(error, dict) else body.get('error_description', ''))
    text = (reason + ' ' + message).lower()
    if 'premium' in text:
        return SpotifyError('Spotify Premium is required to play music on Orion.', 403)
    if 'redirect' in text:
        return SpotifyError(f'In the Spotify developer dashboard, add this exact redirect URI: {REDIRECT_URI}. Save it, then sign in again.')
    if status == 403:
        return SpotifyError('Spotify refused access. In your Spotify app dashboard, open User Management and add the account you use. Then sign in again.', 403)
    if status == 401 or token and reason == 'invalid_grant':
        return SpotifyError('Your Spotify sign-in has expired. Sign in again in Studio.', 401)
    if token and (reason == 'invalid_client' or 'client' in text):
        return SpotifyError('Spotify did not recognise that Client ID. Copy the Client ID from your app dashboard and try again.')
    if status == 404:
        return SpotifyError('Open Spotify on the same Wi-Fi and choose Orion as the speaker. Then try again.', 404)
    if status == 429:
        return SpotifyError('Spotify asked Orion to wait. Please try again shortly.', 429)
    return SpotifyError('Spotify could not complete that request. Please try again.', 502)


class Spotify:
    def __init__(self, path=None, *, send=transport, clock=time.time):
        self.path = path or Path.home() / '.config/orion/spotify.json'
        self.send, self.clock = send, clock
        self.lock = threading.RLock()
        try: self.saved = json.loads(self.path.read_text())
        except (OSError, ValueError): self.saved = {}
        if not isinstance(self.saved, dict): self.saved = {}
        self.pending = None
        self.cached = None
        self.cache_until = self.retry_at = 0.

    @property
    def name(self):
        return self.saved.get('device_name') or os.environ.get('ORION_NAME') or 'Orion'

    def begin(self, client_id, redirect_uri):
        if not isinstance(client_id, str) or not re.fullmatch(r'[0-9a-fA-F]{32}', client_id):
            raise SpotifyError('Paste the 32-character Client ID from your Spotify app dashboard.')
        if redirect_uri != REDIRECT_URI:
            raise SpotifyError(f'Use this redirect URI in your Spotify app: {REDIRECT_URI}')
        with self.lock:
            verifier, state = secrets.token_urlsafe(64), secrets.token_urlsafe(32)
            challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b'=').decode()
            self.pending = dict(client_id=client_id, verifier=verifier, state=state, expires=self.clock()+600)
            url = 'https://accounts.spotify.com/authorize?' + urlencode(dict(client_id=client_id, response_type='code',
                redirect_uri=REDIRECT_URI, state=state, scope=SCOPES, code_challenge_method='S256', code_challenge=challenge))
            return {'url':url, 'redirect_uri':REDIRECT_URI}

    def token(self, fields):
        status, body, _ = self.send('POST', 'https://accounts.spotify.com/api/token',
            {'Content-Type':'application/x-www-form-urlencoded'}, urlencode(fields).encode())
        if status != 200: raise friendly(status, body, token=True)
        if not isinstance(body.get('access_token'), str) or not body.get('expires_in'):
            raise SpotifyError('Spotify returned an incomplete sign-in. Please try again.', 502)
        return body

    def finish(self, code=None, state=None, error=None):
        with self.lock:
            pending = self.pending
            if not pending or not isinstance(state, str) or not hmac.compare_digest(state, pending['state']) or self.clock() >= pending['expires']:
                raise SpotifyError('This Spotify sign-in has expired. Press Sign in with Spotify to start again.')
            self.pending = None
            if error:
                raise SpotifyError('Spotify sign-in was cancelled. Add your account in your app dashboard under User Management if Spotify refused access.')
            if not isinstance(code, str) or not code or len(code) > 8192: raise SpotifyError('Spotify did not return a sign-in code. Please try again.')
            result = self.token(dict(grant_type='authorization_code', code=code, client_id=pending['client_id'],
                redirect_uri=REDIRECT_URI, code_verifier=pending['verifier']))
            if not isinstance(result.get('refresh_token'), str): raise SpotifyError('Spotify did not complete sign-in. Please try again.', 502)
            saved = dict(self.saved, client_id=pending['client_id'], refresh_token=result['refresh_token'],
                access_token=result['access_token'], expires_at=self.clock()+result['expires_in'], connect_revision=secrets.token_hex(16))
            # Check membership before replacing an existing lamp sign-in.
            status, account, _ = self.send('GET', 'https://api.spotify.com/v1/me', {'Authorization':'Bearer '+saved['access_token']}, None)
            if status != 200: raise friendly(status, account)
            if account.get('product') != 'premium': raise SpotifyError('Spotify Premium is required to play music on Orion.', 403)
            save(self.path, saved); self.saved = saved; self.cached = None
            return {'signed_in':True, 'name':self.name}

    def api(self, method, path, body=None, *, retry=True):
        if not self.saved.get('refresh_token'): raise SpotifyError('Sign in with Spotify in Studio to use music by voice.', 401)
        if self.clock() < self.retry_at: raise friendly(429, {})
        if self.clock() >= self.saved.get('expires_at', 0)-30:
            result = self.token(dict(grant_type='refresh_token', refresh_token=self.saved['refresh_token'], client_id=self.saved['client_id']))
            self.saved.update(access_token=result['access_token'], expires_at=self.clock()+result['expires_in'])
            if result.get('refresh_token'): self.saved['refresh_token'] = result['refresh_token']
            save(self.path, self.saved)
        headers = {'Authorization':'Bearer '+self.saved['access_token']}
        if body is not None: headers['Content-Type'] = 'application/json'
        status, value, response_headers = self.send(method, 'https://api.spotify.com/v1/'+path, headers, json.dumps(body).encode() if body is not None else None)
        if status == 401 and retry:
            self.saved['expires_at'] = 0
            return self.api(method, path, body, retry=False)
        if status == 429:
            try: delay = max(1, min(3600, int(response_headers.get('Retry-After', '5'))))
            except (ValueError, TypeError): delay = 5
            self.retry_at = self.clock()+delay
        if status not in (200, 201, 202, 204): raise friendly(status, value)
        return value

    def device(self):
        devices = self.api('GET', 'me/player/devices').get('devices', [])
        expected = hashlib.sha1(self.name.encode()).hexdigest()
        device = next((d for d in devices if d.get('id') == expected), None)
        device = device or next((d for d in devices if d.get('name') == self.name and not d.get('is_restricted')), None)
        if not device or not device.get('id'):
            raise SpotifyError(f'Open Spotify on the same Wi-Fi and choose {self.name} as the speaker. Then try again.', 404)
        return device['id']

    def status(self):
        with self.lock:
            base = {'signed_in':bool(self.saved.get('refresh_token')), 'client_id':self.saved.get('client_id', ''),
                'name':self.name, 'redirect_uri':REDIRECT_URI, 'now_playing':None, 'error':None}
            if not base['signed_in']: return base
            if self.cached and self.clock() < self.cache_until: return dict(base, **self.cached)
            try:
                player = self.api('GET', 'me/player')
                item, device = player.get('item') or {}, player.get('device') or {}
                images = (item.get('album') or {}).get('images') or []
                playing = None
                if item and device.get('name') == self.name:
                    playing = {'title':item.get('name',''), 'artist':', '.join(a.get('name','') for a in item.get('artists', [])),
                        'cover':images[-1].get('url') if images else None, 'playing':bool(player.get('is_playing'))}
                self.cached = {'now_playing':playing, 'error':None}
            except SpotifyError as error: self.cached = {'now_playing':None, 'error':str(error)}
            self.cache_until = self.clock()+1
            return dict(base, **self.cached)

    def command(self, payload):
        if not isinstance(payload, dict): raise SpotifyError('Choose a music action.')
        action = payload.get('action')
        with self.lock:
            if action == 'status' and set(payload) == {'action'}:
                if not self.saved.get('refresh_token'): raise SpotifyError('Sign in with Spotify in Studio to use music by voice.', 401)
                result = self.status()
                if result['error']: raise SpotifyError(result['error'], 502)
                return result
            if action == 'disconnect' and set(payload) == {'action'}:
                self.saved = {key:value for key,value in self.saved.items() if key in ('device_name','connect_revision')}
                save(self.path, self.saved); self.cached = self.pending = None; return self.status()
            if action == 'play':
                query, kind = payload.get('query'), payload.get('kind','track')
                if set(payload)-{'action','query','kind'} or not isinstance(query,str) or not query.strip() or len(query)>500 or kind not in ('track','artist','album','playlist'):
                    raise SpotifyError('Choose a song, artist, album or playlist to play.')
                found = self.api('GET','search?'+urlencode(dict(q=query.strip(), type=kind, limit=5)))
                items = [item for item in found.get(kind+'s',{}).get('items',[]) if item and item.get('uri')]
                if not items: raise SpotifyError("Spotify couldn't find that. Try the title and artist.", 404)
                item = items[0]
                body = {'uris':[item['uri']]} if kind == 'track' else {'context_uri':item['uri']}
                self.api('PUT','me/player/play?'+urlencode({'device_id':self.device()}), body)
                self.cached = None
                return {'ok':True, 'title':item.get('name',''), 'kind':kind}
            routes = {'pause':('PUT','pause'), 'resume':('PUT','play'), 'next':('POST','next'), 'previous':('POST','previous')}
            if action not in routes or set(payload) != {'action'}: raise SpotifyError('Choose play, pause, resume, skip or previous.')
            method, route = routes[action]
            self.api(method,'me/player/'+route+'?'+urlencode({'device_id':self.device()}))
            self.cached = None
            return {'ok':True, 'action':action}
