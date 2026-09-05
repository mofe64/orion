import asyncio
import json
import unittest
from orion_voice_worker.processor import ProcessingOwner
from orion_voice_worker.protocol import event


class Gateway:
    def __init__(self): self.calls=[]; self.playing=False; self.finished=False
    async def request(self, path, body=None, **kwargs):
        self.calls.append((path, body))
        if path == '/api/v2/speech/stream': return {'run_id': 7}
        if path.endswith('/end'): self.finished=True; return {}
        if path == '/api/v2/speech/7':
            return {'state': 'completed' if self.finished else 'playing' if self.playing else 'queued',
                    'first_playback_ms': 250 if self.playing else None}
        return {}


class ProcessorTests(unittest.IsolatedAsyncioTestCase):
    async def test_headless_owner_uploads_buffers_and_acknowledges_actual_playback(self):
        gateway=Gateway(); events=[]
        async def publish(raw): events.append(json.loads(raw))
        owner=ProcessingOwner('a'*32, gateway, publish)
        await owner.recv() # worker handshake
        try:
            await owner.send(event('speech.chunk', requestId=1, sequence=0, samples=4800))
            await owner.send(bytes(9600))
            await asyncio.sleep(.02)
            self.assertTrue(owner.controls.empty(), 'buffering is not playback')
            self.assertFalse(any(e['type']=='speech.started' for e in events))
            gateway.playing=True
            started=json.loads(await asyncio.wait_for(owner.recv(), 1))
            self.assertEqual(started['type'], 'playback.started')
            await owner.send(event('speech.end', requestId=1, sequence=1, synthesisMs=50))
            done=json.loads(await asyncio.wait_for(owner.recv(), 1))
            self.assertEqual(done['type'], 'playback.finished')
            self.assertEqual(done['requestId'], 1)
            self.assertFalse(any(e['type'] in {'speech.chunk','speech.end'} for e in events))
        finally: await owner.close()

    async def test_disconnect_cancels_owned_pi_run(self):
        gateway=Gateway()
        async def publish(raw): pass
        owner=ProcessingOwner('a'*32, gateway, publish)
        await owner.send(event('speech.chunk', requestId=2, sequence=0, samples=100))
        await owner.send(bytes(200))
        await owner.close()
        self.assertIn(('/api/v2/operations', {'operation':'cancel','kind':'speech','run_id':7}), gateway.calls)

    async def test_panel_detaches_but_app_exit_stops_processing(self):
        from unittest.mock import patch
        from types import SimpleNamespace
        from websockets.asyncio.client import connect
        from orion_voice_worker.processor import serve
        import socket
        with socket.socket() as reservation:
            reservation.bind(('127.0.0.1', 0)); port=reservation.getsockname()[1]
        trigger=asyncio.Event(); completed=asyncio.Event(); calls=[]
        gateway=Gateway(); gateway.playing=True
        async def handle(owner, *args):
            calls.append('pipeline')
            await owner.recv()
            await owner.send(event('ready', protocol=7))
            await trigger.wait()
            await owner.send(event('speech.chunk', requestId=1, sequence=0, samples=100))
            await owner.send(bytes(200))
            await owner.send(event('speech.end', requestId=1, sequence=1, synthesisMs=1))
            while json.loads(await owner.recv())['type'] != 'playback.finished': pass
            completed.set()
            await asyncio.Future()
        async def mute(config): return {'type':'microphone.status','muted':False}
        config=dict(port=port, token='a'*32, gateway_url='http://fake', pi_url='ws://fake', pi_token='b'*32)
        with patch('orion_voice_worker.processor.load_models', return_value=SimpleNamespace(agent=SimpleNamespace(close=lambda:None))), \
             patch('orion_voice_worker.processor.Gateway', return_value=gateway), \
             patch('orion_voice_worker.processor.microphone', mute), \
             patch('orion_voice_worker.processor.handle_connection', handle):
            gateway.executor=SimpleNamespace(shutdown=lambda **kwargs:None)
            parent_closed=asyncio.get_running_loop().create_future()
            task=asyncio.create_task(serve(config, parent_closed))
            try:
                for _ in range(100):
                    try:
                        ui=await connect(f'ws://127.0.0.1:{port}'); break
                    except OSError: await asyncio.sleep(.01)
                else: self.fail('Studio worker did not bind')
                await ui.send(event('hello', protocol=7, token='a'*32))
                await asyncio.wait_for(ui.recv(), 1)
                await ui.close()
                trigger.set()
                await asyncio.wait_for(completed.wait(), 2)
                self.assertEqual(calls, ['pipeline'])
                self.assertTrue(any(path.endswith('/end') for path, _ in gateway.calls))
                self.assertFalse(task.done())
                parent_closed.set_result(None)
                await asyncio.wait_for(task, 1)
                self.assertTrue(task.done())
            finally:
                task.cancel()
                await asyncio.gather(task, return_exceptions=True)

