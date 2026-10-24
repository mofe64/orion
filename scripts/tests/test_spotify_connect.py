import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import struct
import unittest
from unittest.mock import patch, Mock

SCRIPTS=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(SCRIPTS))
from spotify_connect import command, environment, forward_pcm
from pi_service_config import services_for_release


class SpotifyConnectTests(unittest.TestCase):
    def test_reconnect_at_every_source_offset_preserves_stereo_sample_boundaries(self):
        original = b''.join(struct.pack('<hh', i, -i) for i in range(1, 30))
        for offset in range(4):
            for partial_send in range(4):
                with self.subTest(offset=offset, partial_send=partial_send):
                    # The failed connection can consume any prefix before sendall
                    # raises. Outage reads end at each possible source alignment.
                    chunks = [original[:8], original[8:12+offset], original[12+offset:19],
                              original[19:23], original[23:]]
                    process = Mock()
                    process.poll.side_effect = [None] * len(chunks) + [0]
                    process.stdout.read1.side_effect = chunks
                    first, second = Mock(), Mock()
                    discarded = bytearray()
                    def fail(data):
                        discarded.extend(data[:partial_send])
                        raise OSError('disconnected part way through sendall')
                    first.sendall.side_effect = fail
                    # First connect, then failure schedules retry at 1.1 s;
                    # the second read is discarded before reconnecting at 2 s.
                    with patch('spotify_connect.socket.socket', side_effect=[first, second]), \
                         patch('spotify_connect.select.select', return_value=([process.stdout], [], [])), \
                         patch('spotify_connect.time.monotonic', side_effect=[0., 1., 1.05, 2.]), \
                         patch('spotify_connect.read_settings', return_value={}):
                        forward_pcm(process, Path('/music.sock'), Path('/settings'), {})
                    actual = b''.join(call.args[0] for call in second.sendall.call_args_list)
                    boundary = (12 + offset + 3) // 4 * 4
                    self.assertEqual(actual, original[boundary:])
                    self.assertEqual(list(struct.iter_unpack('<hh', actual)), list(struct.iter_unpack('<hh', original[boundary:])))
                    for call in second.sendall.call_args_list:
                        self.assertEqual(len(call.args[0]) % 4, 0)
                    first.close.assert_called_once()

    def test_pcm_bridge_reconnects_after_a_stall_without_stopping_librespot(self):
        process=Mock();process.poll.side_effect=[None,None,None,None,0]
        process.stdout.read1.side_effect=[b'old',b'during-outage',b'new',b'last']
        first,second=Mock(),Mock()
        first.sendall.side_effect=TimeoutError('USB audio stalled')
        with patch('spotify_connect.socket.socket',side_effect=[first,second]), patch('spotify_connect.select.select',return_value=([process.stdout],[],[])), patch('spotify_connect.time.monotonic',side_effect=[0.,0.,1.,2.]), patch('spotify_connect.read_settings',return_value={}):
            forward_pcm(process,Path('/music.sock'),Path('/settings'),{})
        first.close.assert_called_once()
        second.connect.assert_called_once_with('/music.sock')
        self.assertTrue(second.sendall.called)
        process.terminate.assert_not_called()

    def test_service_allows_interface_discovery_in_its_address_family_sandbox(self):
        template = (SCRIPTS / 'systemd/orion-spotify.service.in').read_text()
        families = next(line.split('=', 1)[1].split() for line in template.splitlines()
                        if line.startswith('RestrictAddressFamilies='))
        # glibc getifaddrs(), used by libmdns, opens a netlink socket.
        self.assertIn('AF_NETLINK', families)
        self.assertTrue({'AF_INET', 'AF_INET6', 'AF_UNIX'}.issubset(families))

    def test_studio_sign_in_bootstraps_connect_without_tokens_in_arguments(self):
        with patch.dict('os.environ', {}, clear=True), patch('spotify_connect.time.time',return_value=100):
            self.assertEqual(environment({'access_token':'private-token','expires_at':200}), {'LIBRESPOT_ACCESS_TOKEN':'private-token'})
            self.assertEqual(environment({'access_token':'expired-token','expires_at':50}), {})
        self.assertNotIn('private-token',command(Path('/receiver'),'Orion',Path('/cache')))
    def test_receiver_uses_pipe_unity_and_discovery_without_credentials(self):
        args=command(Path('/release/native/librespot'),'Orion',Path('/private/cache'))
        self.assertEqual(args[args.index('--backend')+1],'pipe')
        self.assertEqual(args[args.index('--format')+1],'S16')
        self.assertEqual(args[args.index('--volume-ctrl')+1],'fixed')
        self.assertEqual(args[args.index('--initial-volume')+1],'100')
        self.assertNotIn('--disable-discovery',args)
        for value in ['alsa','--username','--password','--access-token']: self.assertNotIn(value,args)

    def test_source_pin_is_a_full_checksum_and_build_is_locked_without_alsa(self):
        pins=json.loads((SCRIPTS/'release-spotify.json').read_text())
        self.assertRegex(pins['sha256'],r'^[0-9a-f]{64}$')
        self.assertIn('/v'+pins['version'],pins['url'])
        self.assertNotIn('alsa-backend',pins['features'])
        builder=(SCRIPTS/'build_spotify.py').read_text()
        self.assertIn("'--locked'",builder); self.assertIn("'--no-default-features'",builder)
        template=(SCRIPTS/'systemd/orion-spotify.service.in').read_text()
        self.assertIn('WantedBy=multi-user.target',template)
        self.assertIn('scripts/spotify_connect.py',template)


if __name__=='__main__': unittest.main()
