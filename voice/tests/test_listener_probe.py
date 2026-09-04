import importlib.util
import json
from pathlib import Path
import unittest
from websockets.asyncio.server import serve

spec = importlib.util.spec_from_file_location('probe', Path(__file__).resolve().parents[2] / 'scripts/check_pi_listener.py')
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)

class ListenerProbeTests(unittest.IsolatedAsyncioTestCase):
    async def test_expected_authentication_rejection_is_ready(self):
        async def listener(ws):
            hello = json.loads(await ws.recv())
            self.assertEqual(hello['protocol'], 1)
            self.assertEqual(hello['token'], 'invalid-deployment-probe')
            await ws.close(4003, 'Invalid listener handshake')
        async with serve(listener, '127.0.0.1', 0) as server:
            await probe.main(f'ws://127.0.0.1:{server.sockets[0].getsockname()[1]}')

    async def test_accepting_bad_authentication_fails_deployment_check(self):
        async def listener(ws):
            await ws.recv()
            await ws.send('{"type":"ready"}')
        async with serve(listener, '127.0.0.1', 0) as server:
            with self.assertRaisesRegex(RuntimeError, 'accepted an invalid token'):
                await probe.main(f'ws://127.0.0.1:{server.sockets[0].getsockname()[1]}')
