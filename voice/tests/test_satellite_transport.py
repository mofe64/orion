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
            expressions=[]
            async def daemon(command, path): expressions.append(command); return {"ok": True}
            with patch('orion_voice.satellite.RustpotterWakeDetector',FakeWake),patch('orion_voice.satellite.StereoCapture',FakeCapture),patch('orion_voice.satellite.daemon_command',daemon):
                task=asyncio.create_task(serve(args))
                try:
                    for _ in range(100):
                        try:
                            first=await connect(f'ws://127.0.0.1:{port}')
                            break
                        except OSError: await asyncio.sleep(.01)
                    else: self.fail('Listener did not start')
                    await asyncio.sleep(.04)
                    self.assertTrue(any(command.endswith(' wake') for command in expressions), 'wake feedback must precede processor authentication')
                    async with first:
                        await first.send(json.dumps(dict(type='hello',protocol=1,token='bad')))
                        with self.assertRaises(ConnectionClosed): await first.recv()
                    self.assertTrue(FakeCapture.instances[-1].opened)
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
                        await asyncio.sleep(.01)
                        identity=candidate['sessionId']
                        self.assertEqual(expressions.count(f'voice {identity} wake'), 1)
                        self.assertEqual(expressions.count(f'voice {identity} endpoint'), 1)
                    await asyncio.sleep(.03)
                    self.assertTrue(FakeCapture.instances[-1].opened)
                    async with connect(f'ws://127.0.0.1:{port}') as controller:
                        await controller.send(json.dumps(dict(type='hello', protocol=1, token='a'*32, role='control')))
                        self.assertFalse(json.loads(await controller.recv())['muted'])
                        await controller.send(json.dumps(dict(type='microphone.mute', muted=True)))
                        self.assertTrue(json.loads(await controller.recv())['muted'])
                        self.assertFalse(FakeCapture.instances[-1].opened)
                        self.assertTrue(json.loads((Path(directory)/'microphone.json').read_text())['muted'])
                    # Restart reads the preference before opening the microphone.
                    task.cancel()
                    with suppress(asyncio.CancelledError): await task
                    task=asyncio.create_task(serve(args))
                    await asyncio.sleep(.05)
                    self.assertFalse(FakeCapture.instances[-1].opened)
                finally:
                    task.cancel()
                    with suppress(asyncio.CancelledError): await task

    async def test_mute_waits_for_inflight_open_before_acknowledging(self):
        import threading
        started=threading.Event(); release=threading.Event()
        class SlowCapture(FakeCapture):
            def open(self):
                started.set()
                release.wait(2)
                super().open()
        with tempfile.TemporaryDirectory() as directory:
            token_file=Path(directory)/'token'; token_file.write_text('a'*32)
            with socket.socket() as reservation:
                reservation.bind(('127.0.0.1',0)); port=reservation.getsockname()[1]
            args=SimpleNamespace(token_file=token_file,host='127.0.0.1',port=port,
                wake_model=Path('unused'),threshold=.4,device='fake',mic_spacing=0,channel_sign=0,
                daemon_socket=str(Path(directory)/'missing.sock'))
            with patch('orion_voice.satellite.RustpotterWakeDetector',FakeWake), \
                 patch('orion_voice.satellite.StereoCapture',SlowCapture):
                task=asyncio.create_task(serve(args))
                try:
                    for _ in range(100):
                        if started.is_set(): break
                        await asyncio.sleep(.01)
                    async with connect(f'ws://127.0.0.1:{port}') as client:
                        await client.send(json.dumps(dict(type='hello',protocol=1,token='a'*32,role='control')))
                        await client.recv()
                        await client.send(json.dumps(dict(type='microphone.mute',muted=True)))
                        acknowledged=asyncio.create_task(client.recv())
                        await asyncio.sleep(.03)
                        self.assertFalse(acknowledged.done())
                        release.set()
                        result=json.loads(await asyncio.wait_for(acknowledged,1))
                        self.assertTrue(result['muted'])
                        self.assertFalse(FakeCapture.instances[-1].opened)
                finally:
                    release.set()
                    task.cancel()
                    await asyncio.gather(task,return_exceptions=True)

    async def test_offline_capture_ends_with_unavailable_and_keeps_listening(self):
        with tempfile.TemporaryDirectory() as directory:
            token_file=Path(directory)/'token'; token_file.write_text('a'*32)
            args=SimpleNamespace(token_file=token_file,host='127.0.0.1',port=0,
                wake_model=Path('unused'),threshold=.4,device='fake',mic_spacing=0,channel_sign=0,
                daemon_socket=str(Path(directory)/'missing.sock'))
            expressions=[]
            async def daemon(command,path): expressions.append(command); return {'ok':True}
            with patch('orion_voice.satellite.RustpotterWakeDetector',FakeWake), \
                 patch('orion_voice.satellite.StereoCapture',FakeCapture), \
                 patch('orion_voice.satellite.daemon_command',daemon):
                task=asyncio.create_task(serve(args))
                try:
                    for _ in range(100):
                        if any(command.endswith(' unavailable') for command in expressions): break
                        await asyncio.sleep(.01)
                    else: self.fail('Offline capture never reported unavailability')
                    wake=next(command for command in expressions if command.endswith(' wake'))
                    identity=wake.split()[1]
                    self.assertEqual(expressions.count(f'voice {identity} unavailable'),1)
                    self.assertNotIn(f'voice {identity} endpoint',expressions)
                    self.assertTrue(FakeCapture.instances[-1].opened)
                finally:
                    task.cancel()
                    await asyncio.gather(task,return_exceptions=True)
