import json
import os
from pathlib import Path
import socket
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from gateway import GatewayError, voice_service_request


class VoiceBridgeTests(unittest.TestCase):
    def test_only_onboard_controls_are_forwarded(self):
        for method in ('start', 'save_pairing', 'load_pairing', 'forget_pairing', 'exec'):
            with self.subTest(method=method), self.assertRaises(GatewayError):
                voice_service_request({'method': method})

    def test_loopback_credentials_never_reach_studio(self):
        with tempfile.TemporaryDirectory() as folder, socket.socket() as server:
            server.bind(('127.0.0.1', 0)); server.listen()
            connection = {'protocol': 1, 'address': '127.0.0.1:' + str(server.getsockname()[1]), 'token': 'private-owner-token'}
            Path(folder, 'connection.json').write_text(json.dumps(connection))
            received = []
            def serve():
                with server.accept()[0] as client, client.makefile('rb') as reader:
                    received.append(json.loads(reader.readline()))
                    client.sendall(json.dumps({'ok': True, 'result': {'url': 'ws://127.0.0.1:12345', 'token': 'private-observer-token', 'asrModel': 'qwen'}}).encode() + b'\n')
            thread = threading.Thread(target=serve); thread.start()
            with patch.dict(os.environ, ORION_STUDIO_SERVICE_HOME=folder):
                result = voice_service_request({'method': 'start_saved'})
            thread.join(timeout=2)
            self.assertEqual(result, {'url': '/api/v2/voice/events', 'asrModel': 'qwen'})
            self.assertEqual(received[0]['token'], 'private-owner-token')
            self.assertEqual(received[0]['request'], {'method': 'start_saved'})

    def test_history_is_read_from_the_pi_service(self):
        with tempfile.TemporaryDirectory() as folder, socket.socket() as server:
            server.bind(('127.0.0.1', 0)); server.listen()
            connection = {'protocol': 1, 'address': '127.0.0.1:' + str(server.getsockname()[1]), 'token': 'private-owner-token'}
            Path(folder, 'connection.json').write_text(json.dumps(connection))
            received = []
            def serve():
                with server.accept()[0] as client, client.makefile('rb') as reader:
                    received.append(json.loads(reader.readline()))
                    client.sendall(json.dumps({'ok': True, 'result': {'items': [], 'nextCursor': None}}).encode() + b'\n')
            thread = threading.Thread(target=serve); thread.start()
            with patch.dict(os.environ, ORION_STUDIO_SERVICE_HOME=folder):
                result = voice_service_request({'method': 'history', 'params': {'session_id': None, 'before': None}})
            thread.join(timeout=2)
            self.assertEqual(result, {'items': [], 'nextCursor': None})
            self.assertEqual(received[0]['request']['method'], 'history')

    def test_non_loopback_owner_address_is_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            Path(folder, 'connection.json').write_text(json.dumps({'protocol': 1, 'address': '192.0.2.1:1234', 'token': 'private'}))
            with patch.dict(os.environ, ORION_STUDIO_SERVICE_HOME=folder), self.assertRaises(GatewayError):
                voice_service_request({'method': 'status'})


if __name__ == '__main__':
    unittest.main()
