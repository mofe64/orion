import base64
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from urllib.parse import parse_qs, urlparse

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from spotify import Spotify, SpotifyError, REDIRECT_URI, friendly


class SpotifyTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory(); self.addCleanup(self.folder.cleanup)
        self.path = Path(self.folder.name) / 'spotify.json'
        self.now = 1000.; self.calls = []; self.replies = []
        def send(method, url, headers, body):
            self.calls.append((method, url, headers, body))
            return self.replies.pop(0)
        self.spotify = Spotify(self.path, send=send, clock=lambda:self.now)

    def signed_in(self):
        self.spotify.saved = {'client_id':'a'*32,'refresh_token':'lamp-refresh','access_token':'lamp-access','expires_at':5000}

    def reply(self, value=None, status=200, headers=None):
        self.replies.append((status,value or {},headers or {}))

    def test_pkce_exact_redirect_state_one_use_and_private_lamp_storage(self):
        begin = self.spotify.begin('a'*32, REDIRECT_URI)
        fields = parse_qs(urlparse(begin['url']).query)
        pending = self.spotify.pending
        expected = base64.urlsafe_b64encode(hashlib.sha256(pending['verifier'].encode()).digest()).rstrip(b'=').decode()
        self.assertEqual(fields['code_challenge'], [expected]); self.assertEqual(fields['code_challenge_method'], ['S256'])
        self.assertEqual(fields['redirect_uri'], [REDIRECT_URI]); self.assertNotIn('client_secret', fields)
        self.assertIn('streaming', fields['scope'][0].split())
        with self.assertRaises(SpotifyError): self.spotify.finish('code','wrong-state')
        self.assertIsNotNone(self.spotify.pending)
        self.reply({'access_token':'lamp-access','refresh_token':'lamp-refresh','expires_in':3600})
        self.reply({'product':'premium'})
        self.assertTrue(self.spotify.finish('code', pending['state'])['signed_in'])
        token = parse_qs(self.calls[0][3].decode()); self.assertEqual(token['code_verifier'], [pending['verifier']])
        self.assertNotIn('client_secret',token)
        self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)
        self.assertEqual(json.loads(self.path.read_text())['refresh_token'],'lamp-refresh')
        self.assertTrue(json.loads(self.path.read_text())['connect_revision'])
        with self.assertRaises(SpotifyError): self.spotify.finish('code',pending['state'])
        self.reply(status=204)
        public = json.dumps(self.spotify.status()); self.assertNotIn('lamp-refresh',public); self.assertNotIn('lamp-access',public)
        self.assertTrue(Spotify(self.path).saved['refresh_token'])

    def test_client_id_loopback_expiry_and_cancel_are_plain_errors(self):
        for client in ['', 'a'*31, 'g'*32, None]:
            with self.assertRaisesRegex(SpotifyError,'Client ID'): self.spotify.begin(client, REDIRECT_URI)
        with self.assertRaisesRegex(SpotifyError,'address in your Spotify app matches the one in Studio'): self.spotify.begin('a'*32,'http://localhost:49687/callback')
        self.spotify.begin('a'*32,REDIRECT_URI); state=self.spotify.pending['state']; self.now+=601
        with self.assertRaisesRegex(SpotifyError,'expired'): self.spotify.finish('x',state)
        self.spotify.begin('a'*32,REDIRECT_URI); state=self.spotify.pending['state']
        with self.assertRaisesRegex(SpotifyError,'cancelled'): self.spotify.finish(state=state,error='access_denied')

    def test_non_premium_and_missing_user_do_not_replace_credentials(self):
        for account, status, message in [({'product':'free'},200,'Premium'), ({},403,'User Management')]:
            self.spotify.begin('a'*32,REDIRECT_URI); state=self.spotify.pending['state']
            self.reply({'access_token':'access','refresh_token':'refresh','expires_in':3600}); self.reply(account,status)
            with self.assertRaisesRegex(SpotifyError,message): self.spotify.finish('code',state)
            self.assertFalse(self.path.exists())
        self.assertEqual(str(friendly(400,{'error':'invalid_request','error_description':'redirect URI mismatch'},token=True)),
                         'Check that the address in your Spotify app matches the one in Studio, then sign in again.')
        self.assertIn('Client ID',str(friendly(400,{'error':'invalid_client'},token=True)))
        self.assertIn('Premium',str(friendly(403,{'error':{'reason':'PREMIUM_REQUIRED'}})))

    def test_search_and_start_each_kind_on_orion_device(self):
        self.signed_in()
        for kind in ['track','artist','album','playlist']:
            self.reply({kind+'s':{'items':[None,{'uri':'spotify:'+kind+':one','name':'Requested music'}]}})
            self.reply({'devices':[{'id':'other','name':'Kitchen'},{'id':'lamp','name':'Orion'}]})
            self.reply(status=204)
            result = self.spotify.command({'action':'play','query':'A title & artist','kind':kind})
            self.assertTrue(result['ok']); self.assertEqual(result['title'],'Requested music')
            self.assertEqual(parse_qs(urlparse(self.calls[-3][1]).query)['q'],['A title & artist'])
            method,url,headers,body = self.calls[-1]
            self.assertEqual(method,'PUT'); self.assertTrue(url.endswith('me/player/play?device_id=lamp'))
            self.assertEqual(json.loads(body), {'uris':['spotify:track:one']} if kind=='track' else {'context_uri':'spotify:'+kind+':one'})

    def test_playback_commands_never_target_the_other_speaker(self):
        self.signed_in()
        for action,method,route in [('pause','PUT','pause'),('resume','PUT','play'),('next','POST','next'),('previous','POST','previous')]:
            self.reply({'devices':[{'id':'lamp','name':'Orion'}]}); self.reply(status=204)
            self.spotify.command({'action':action})
            self.assertEqual(self.calls[-1][:2], (method,'https://api.spotify.com/v1/me/player/'+route+'?device_id=lamp'))
        self.reply({'devices':[{'id':'other','name':'Kitchen'}]})
        with self.assertRaisesRegex(SpotifyError,'choose Orion'): self.spotify.command({'action':'pause'})

    def test_refresh_rotation_stays_on_lamp_and_rate_limit_does_not_hammer_spotify(self):
        self.signed_in(); self.spotify.saved['expires_at']=0
        self.reply({'access_token':'new-access','refresh_token':'new-refresh','expires_in':3600}); self.reply(status=204)
        self.spotify.api('PUT','me/player/pause')
        self.assertEqual(json.loads(self.path.read_text())['refresh_token'],'new-refresh')
        self.assertNotIn('client_secret',self.calls[0][3].decode())
        self.reply(status=429,headers={'Retry-After':'30'})
        with self.assertRaisesRegex(SpotifyError,'wait'): self.spotify.api('GET','me/player')
        calls=len(self.calls)
        with self.assertRaises(SpotifyError): self.spotify.api('GET','me/player')
        self.assertEqual(len(self.calls),calls)

    def test_now_playing_empty_cache_and_sign_out(self):
        self.signed_in()
        item={'name':'Song','artists':[{'name':'Artist'}],'album':{'images':[{'url':'https://i.scdn.co/image/one'}]}}
        self.reply({'item':item,'device':{'name':'Orion'},'is_playing':True})
        self.assertEqual(self.spotify.command({'action':'status'})['now_playing']['artist'],'Artist')
        self.spotify.status(); self.assertEqual(len(self.calls),1)
        self.now+=2; self.reply(status=204); self.assertIsNone(self.spotify.status()['now_playing'])
        self.assertFalse(self.spotify.command({'action':'disconnect'})['signed_in'])
        self.assertNotIn('refresh_token',self.path.read_text())
        with self.assertRaisesRegex(SpotifyError,'Sign in'): self.spotify.command({'action':'pause'})


if __name__ == '__main__': unittest.main()
