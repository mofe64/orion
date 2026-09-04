import asyncio
from contextlib import suppress
import json
from pathlib import Path
import socket
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import numpy as np
from websockets.asyncio.client import connect
from websockets.exceptions import ConnectionClosed
from orion_voice.satellite import serve

class FakeWake:
    provider='rustpotter';model_name='pi.rpw';threshold=.4
    def __init__(self,*args): self.reset()
    def reset(self): self.frames=0
    def process(self,pcm):
        self.frames+=1
        return SimpleNamespace(name='hey_orion',score=.8) if self.frames==5 else None

class FakeCapture:
    instances=[]
    def __init__(self,*args): self.opened=False; self.frames=0; self.instances.append(self)
    def open(self): self.opened=True;self.frames=0
    def close(self): self.opened=False
    def read(self):
        time.sleep(.005)
        self.frames+=1
        return np.full((320,2),2000 if self.frames<25 else 0,dtype='<i2').tobytes()

class ListenerTransportTests(unittest.IsolatedAsyncioTestCase):
    async def test_authentication_exclusive_capture_utterance_and_disconnect_cleanup(self):
        with tempfile.TemporaryDirectory() as directory:
            token_file=Path(directory)/'token';token_file.write_text('a'*32)
            with socket.socket() as reservation:
                reservation.bind(('127.0.0.1',0));port=reservation.getsockname()[1]
            args=SimpleNamespace(token_file=token_file,host='0.0.0.0',port=port,
                wake_model=Path('unused'),threshold=.4,device='fake',mic_spacing=0,channel_sign=0,
                daemon_socket=str(Path(directory)/'no-robot.sock'))
            with patch('orion_voice.satellite.RustpotterWakeDetector',FakeWake),patch('orion_voice.satellite.StereoCapture',FakeCapture):
                task=asyncio.create_task(serve(args))
                try:
                    for _ in range(100):
                        try:
                            first=await connect(f'ws://127.0.0.1:{port}')
                            break
                        except OSError: await asyncio.sleep(.01)
                    else: self.fail('Listener did not start')
                    async with first:
                        await first.send(json.dumps(dict(type='hello',protocol=1,token='bad')))
                        with self.assertRaises(ConnectionClosed): await first.recv()
                    self.assertFalse(FakeCapture.instances[-1].opened)
                    async with connect(f'ws://127.0.0.1:{port}') as client:
                        await client.send(json.dumps(dict(type='hello',protocol=1,token='a'*32)))
                        ready=json.loads(await client.recv())
                        self.assertEqual(ready['wake']['provider'],'rustpotter')
                        self.assertTrue(FakeCapture.instances[-1].opened)
                        async with connect(f'ws://127.0.0.1:{port}') as extra:
                            await extra.send(json.dumps(dict(type='hello',protocol=1,token='a'*32)))
                            with self.assertRaises(ConnectionClosed): await extra.recv()
                        candidate=json.loads(await asyncio.wait_for(client.recv(),2))
                        self.assertEqual(candidate['type'],'wake.candidate')
                        utterance=json.loads(await asyncio.wait_for(client.recv(),2))
                        audio=await client.recv()
                        self.assertEqual(utterance['bytes'],len(audio))
                        self.assertEqual(utterance['sessionId'],candidate['sessionId'])
                    for _ in range(100):
                        if not FakeCapture.instances[-1].opened: break
                        await asyncio.sleep(.01)
                    self.assertFalse(FakeCapture.instances[-1].opened)
                finally:
                    task.cancel()
                    with suppress(asyncio.CancelledError): await task
