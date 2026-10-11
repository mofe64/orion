import asyncio
import json
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import AsyncMock, Mock, patch

from orion_voice.satellite import age_attention_command, visual_attention


class VisualAttentionTests(unittest.IsolatedAsyncioTestCase):
    async def test_confirmed_v2_queries_once_after_confirmation_and_v1_keeps_its_side(self):
        import test_satellite_transport as transport
        for hardware in ('v2', 'v1'):
            direction = Mock()
            direction.observation.side_effect = lambda: {'side': 'left', 'confidence': 1., 'observed_at': time.monotonic()}
            query = AsyncMock(return_value='attend_bearing .4 .1 100')
            with patch('orion_voice.satellite.visual_attention', query), patch('orion_voice.satellite.DirectionEstimator', return_value=direction):
                client, identity, expressions = await transport.ListenerTransportTests.acoustic_session(self, {1: .9}, hardware)
                verdict = json.loads(await client.recv())
                self.assertTrue(verdict['accepted'])
                verb = 'attend_bearing' if hardware == 'v2' else 'attend_left'
                for _ in range(200):
                    if any(command.startswith(f'voice {identity} {verb} ') for command in expressions):
                        break
                    await asyncio.sleep(.01)
                else:
                    self.fail(f'Missing {verb}: {expressions}')
                confirmed = expressions.index(f'voice {identity} confirmed')
                attended = next(i for i, command in enumerate(expressions) if command.startswith(f'voice {identity} {verb} '))
                self.assertLess(confirmed, attended)
                if hardware == 'v2':
                    query.assert_awaited_once()
                    full = json.loads(await client.recv()); await client.recv()
                    self.assertEqual(full['purpose'], 'wake_and_command')
                    await client.send(json.dumps({'type': 'wake.confirmed', 'sessionId': identity, 'followup': False}))
                    await asyncio.sleep(.1)
                    query.assert_awaited_once()
                else:
                    query.assert_not_awaited()
                await client.close()

    async def test_unresponsive_vision_is_bounded_and_does_not_block_other_tasks(self):
        async def unavailable(*args, **kwargs):
            await asyncio.Future()
        ticks = []
        async def capture():
            for _ in range(20):
                ticks.append(1)
                await asyncio.sleep(.01)
        started = time.monotonic()
        with patch('orion_voice.satellite.asyncio.open_unix_connection', side_effect=unavailable):
            result, _ = await asyncio.gather(visual_attention('unused', 0), capture())
        self.assertIsNone(result)
        self.assertEqual(len(ticks), 20)
        self.assertLess(time.monotonic() - started, .6)

    async def query(self, value, status=200, delay=0):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'vision.sock'
            requests = []
            async def respond(reader, writer):
                requests.append((await reader.readuntil(b'\r\n\r\n')).decode())
                await asyncio.sleep(delay)
                body = json.dumps(value).encode()
                writer.write(f'HTTP/1.0 {status} response\r\nContent-Length: {len(body)}\r\n\r\n'.encode() + body)
                try:
                    await writer.drain()
                except ConnectionError:
                    pass
                finally:
                    writer.close()
                    await writer.wait_closed()
            server = await asyncio.start_unix_server(respond, path)
            try:
                result = await visual_attention(path, 100, clock=lambda: 2_000_000_000)
                return result, requests
            finally:
                server.close(); await server.wait_closed()

    async def test_queries_metadata_and_sends_bearing_with_capture_age(self):
        value = {'speaker': {'capture_boottime_ns': 1_500_000_000, 'azimuth': .4, 'elevation': .1}}
        command, requests = await self.query(value)
        self.assertEqual(command, 'attend_bearing 0.400000000 0.100000000 500.000')
        self.assertIn('GET /speaker?since_ns=100 ', requests[0])
        self.assertNotIn('frame', requests[0])
        self.assertEqual(age_attention_command('voice owner ' + command, .25),
            'voice owner attend_bearing 0.400000000 0.100000000 750.0')

    async def test_no_face_scans_once_but_off_unavailable_stale_or_invalid_never_turns(self):
        command, _ = await self.query({'speaker': None, 'observed_boottime_ns': 1_900_000_000})
        self.assertEqual(command, 'attend_scan 100.000')
        for status, value in [(503, {'error': {'code': 'camera_off'}}),
                              (503, {'error': {'code': 'detection_unavailable'}}),
                              (200, {'speaker': None}),
                              (200, {'speaker': {'capture_boottime_ns': -2_000_000_000, 'azimuth': 0, 'elevation': 0}}),
                              (200, {'speaker': {'capture_boottime_ns': 1_900_000_000, 'azimuth': float('nan'), 'elevation': 0}})]:
            result, _ = await self.query(value, status)
            self.assertIsNone(result)
        self.assertIsNone(await visual_attention('/nonexistent-vision.sock', 0))

    async def test_tracking_off_returns_no_visual_attention_and_v2_wake_never_turns_or_scans(self):
        attention, requests = await self.query({'error': {'code': 'tracking_off'}}, status=503)
        self.assertIsNone(attention)
        self.assertIn('GET /speaker?', requests[0])
        import test_satellite_transport as transport
        direction = Mock()
        direction.observation.side_effect = lambda: {'side': 'left', 'confidence': 1., 'observed_at': time.monotonic()}
        with patch('orion_voice.satellite.visual_attention', AsyncMock(return_value=attention)) as query, \
             patch('orion_voice.satellite.DirectionEstimator', return_value=direction):
            client, identity, expressions = await transport.ListenerTransportTests.acoustic_session(self, {1: .9}, 'v2')
            self.assertTrue(json.loads(await client.recv())['accepted'])
            full = json.loads(await client.recv()); await client.recv()
            self.assertEqual(full['purpose'], 'wake_and_command')
            for _ in range(200):
                if f'voice {identity} endpoint' in expressions:
                    break
                await asyncio.sleep(.01)
            else:
                self.fail(f'Missing endpoint: {expressions}')
            self.assertIn(f'voice {identity} confirmed', expressions)
            query.assert_awaited_once()
            self.assertFalse(any(' attend_' in command for command in expressions), expressions)
            await client.close()

    def test_v1_queue_age_contract_is_unchanged(self):
        self.assertEqual(age_attention_command('voice owner attend_left 500', .25), 'voice owner attend_left 750.0')
        self.assertEqual(age_attention_command('voice owner attend_right 0', .1), 'voice owner attend_right 100.0')
        self.assertEqual(age_attention_command('voice owner confirmed', .25), 'voice owner confirmed')
        self.assertEqual(age_attention_command('voice owner attend_scan 0', .25), 'voice owner attend_scan 250.0')


if __name__ == '__main__':
    unittest.main()
