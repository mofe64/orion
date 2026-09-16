import asyncio
from contextlib import suppress
import json
import logging
from pathlib import Path
import socket
import tempfile
import threading
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
    async def test_prefix_confirmation_is_ordered_while_capture_continues(self):
        with tempfile.TemporaryDirectory() as directory:
            token_file = Path(directory) / 'token'; token_file.write_text('a' * 32)
            with socket.socket() as reservation:
                reservation.bind(('127.0.0.1', 0)); port = reservation.getsockname()[1]
            args = SimpleNamespace(token_file=token_file, host='127.0.0.1', port=port,
                wake_model=Path('unused'), threshold=.4, device='fake', mic_spacing=0, channel_sign=0,
                daemon_socket=str(Path(directory) / 'no-robot.sock'))
            expressions = []
            async def daemon(command, path):
                expressions.append(command)
                return {'ok': True}
            with patch('orion_voice.satellite.RustpotterWakeDetector', FakeWake), \
                 patch('orion_voice.satellite.StereoCapture', FakeCapture), \
                 patch('orion_voice.satellite.daemon_command', daemon):
                task = asyncio.create_task(serve(args))
                try:
                    for _ in range(100):
                        try:
                            client = await connect(f'ws://127.0.0.1:{port}'); break
                        except OSError: await asyncio.sleep(.01)
                    else: self.fail('Listener did not start')
                    async with client:
                        await client.send(json.dumps(dict(type='hello', protocol=1, token='a'*32, wakePrefix=True)))
                        self.assertTrue(json.loads(await client.recv())['wakePrefix'])
                        candidate = json.loads(await client.recv()); sid = candidate['sessionId']
                        prefix = json.loads(await client.recv()); await client.recv()
                        self.assertEqual(prefix['purpose'], 'wake_prefix')
                        capture = FakeCapture.instances[-1]; before = capture.frames
                        await asyncio.sleep(.08)
                        self.assertGreater(capture.frames, before)
                        await client.send(json.dumps({'type':'wake.verified','sessionId':sid,'accepted':True}))
                        full = json.loads(await asyncio.wait_for(client.recv(), 2)); await client.recv()
                        self.assertEqual(full['purpose'], 'wake_and_command')
                        for _ in range(50):
                            if f'voice {sid} endpoint' in expressions: break
                            await asyncio.sleep(.01)
                        sequence = [f'voice {sid} {event}' for event in ['wake','verify','confirmed','endpoint']]
                        indices = [expressions.index(event) for event in sequence]
                        self.assertEqual(indices, sorted(indices))
                finally:
                    task.cancel(); await asyncio.gather(task, return_exceptions=True)

    async def test_unmute_waits_until_microphone_startup_finishes(self):
        opening, finish = threading.Event(), threading.Event()
        class SlowCapture(FakeCapture):
            def open(self):
                opening.set()
                if not finish.wait(2): raise RuntimeError('Test did not release capture')
                super().open()
        with tempfile.TemporaryDirectory() as directory:
            token_file = Path(directory) / 'token'; token_file.write_text('a' * 32)
            mute_file = Path(directory) / 'muted.json'; mute_file.write_text('{"muted":true}')
            with socket.socket() as reservation:
                reservation.bind(('127.0.0.1', 0)); port = reservation.getsockname()[1]
            args = SimpleNamespace(token_file=token_file, mute_file=mute_file, host='127.0.0.1', port=port,
                wake_model=Path('unused'), threshold=.4, device='fake', mic_spacing=0, channel_sign=0,
                daemon_socket=str(Path(directory) / 'no-robot.sock'))
            with patch('orion_voice.satellite.RustpotterWakeDetector', FakeWake), patch('orion_voice.satellite.StereoCapture', SlowCapture):
                task = asyncio.create_task(serve(args))
                try:
                    for _ in range(100):
                        try:
                            client = await connect(f'ws://127.0.0.1:{port}'); break
                        except OSError: await asyncio.sleep(.01)
                    else: self.fail('Listener did not start')
                    async with client:
                        await client.send(json.dumps(dict(type='hello', protocol=1, token='a'*32, role='control')))
                        self.assertTrue(json.loads(await client.recv())['muted'])
                        await client.send(json.dumps(dict(type='microphone.mute', muted=False)))
                        reply = asyncio.create_task(client.recv())
                        for _ in range(100):
                            if opening.is_set(): break
                            await asyncio.sleep(.01)
                        self.assertTrue(opening.is_set())
                        self.assertFalse(reply.done(), 'Unmute must not report ready before capture opens')
                        finish.set()
                        self.assertFalse(json.loads(await asyncio.wait_for(reply, 2))['muted'])
                        self.assertTrue(SlowCapture.instances[-1].opened)
                finally:
                    finish.set(); task.cancel()
                    await asyncio.gather(task, return_exceptions=True)

    async def test_confirmation_reaches_runtime_before_optional_fresh_attention(self):
        for side, age in [("unknown", 0), ("left", 0), ("right", 4)]:
            with self.subTest(side=side, age=age), tempfile.TemporaryDirectory() as directory:
                class Direction:
                    def __init__(self, *args): pass
                    def reset(self): pass
                    def accept(self, audio): pass
                    def observation(self):
                        return {"side": side, "confidence": 0.9, "observed_at": time.monotonic() - age}
                token_file = Path(directory) / 'token'; token_file.write_text('a' * 32)
                with socket.socket() as reservation:
                    reservation.bind(('127.0.0.1', 0)); port = reservation.getsockname()[1]
                args = SimpleNamespace(token_file=token_file, host='127.0.0.1', port=port,
                    wake_model=Path('unused'), threshold=.4, device='fake', mic_spacing=0, channel_sign=0,
                    daemon_socket=str(Path(directory) / 'no-robot.sock'))
                expressions = []
                async def daemon(command, path):
                    expressions.append(command)
                    return {'ok': True}
                with patch('orion_voice.satellite.RustpotterWakeDetector', FakeWake), \
                     patch('orion_voice.satellite.StereoCapture', FakeCapture), \
                     patch('orion_voice.satellite.DirectionEstimator', Direction), \
                     patch('orion_voice.satellite.daemon_command', daemon):
                    task = asyncio.create_task(serve(args))
                    try:
                        for _ in range(100):
                            try:
                                client = await connect(f'ws://127.0.0.1:{port}')
                                break
                            except OSError: await asyncio.sleep(.01)
                        else: self.fail('Listener did not start')
                        async with client:
                            await client.send(json.dumps(dict(type='hello', protocol=1, token='a' * 32)))
                            await client.recv()
                            candidate = json.loads(await asyncio.wait_for(client.recv(), 2))
                            await client.recv(); await client.recv()
                            identity = candidate['sessionId']
                            await client.send(json.dumps({'type': 'wake.confirmed', 'sessionId': identity, 'followup': True}))
                            for _ in range(100):
                                if f'voice {identity} followup' in expressions: break
                                await asyncio.sleep(.01)
                            else: self.fail('Confirmation was not forwarded')
                            wake = f'voice {identity} wake'
                            endpoint = f'voice {identity} endpoint'
                            confirmed = f'voice {identity} confirmed'
                            self.assertEqual(expressions.count(confirmed), 1)
                            self.assertLess(expressions.index(wake), expressions.index(endpoint))
                            self.assertLess(expressions.index(endpoint), expressions.index(confirmed))
                            attention = [value for value in expressions if f'voice {identity} attend_' in value]
                            if side == 'left':
                                self.assertEqual(len(attention), 1)
                                self.assertLess(expressions.index(confirmed), expressions.index(attention[0]))
                                self.assertLess(float(attention[0].split()[-1]), 3000)
                            else: self.assertEqual(attention, [])
                    finally:
                        task.cancel()
                        await asyncio.gather(task, return_exceptions=True)

    async def test_abrupt_control_disconnect_is_quiet_and_capture_survives(self):
        with tempfile.TemporaryDirectory() as directory:
            token_file = Path(directory) / 'token'; token_file.write_text('a' * 32)
            with socket.socket() as reservation:
                reservation.bind(('127.0.0.1', 0)); port = reservation.getsockname()[1]
            args = SimpleNamespace(token_file=token_file, host='127.0.0.1', port=port,
                wake_model=Path('unused'), threshold=.4, device='fake', mic_spacing=0, channel_sign=0,
                daemon_socket=str(Path(directory) / 'missing.sock'))
            with patch('orion_voice.satellite.RustpotterWakeDetector', FakeWake), \
                 patch('orion_voice.satellite.StereoCapture', FakeCapture), \
                 patch.object(logging.getLogger('websockets.server'), 'error') as errors:
                task = asyncio.create_task(serve(args))
                try:
                    for _ in range(100):
                        try:
                            client = await connect(f'ws://127.0.0.1:{port}')
                            break
                        except OSError:
                            await asyncio.sleep(.01)
                    else:
                        self.fail('Listener did not start')
                    await client.send(json.dumps(dict(type='hello', protocol=1, token='a' * 32, role='control')))
                    self.assertFalse(json.loads(await client.recv())['muted'])
                    client.transport.abort()
                    await client.wait_closed()
                    await asyncio.sleep(.03)
                    self.assertTrue(FakeCapture.instances[-1].opened)
                    async with connect(f'ws://127.0.0.1:{port}') as controller:
                        await controller.send(json.dumps(dict(type='hello', protocol=1, token='a' * 32, role='control')))
                        self.assertFalse(json.loads(await controller.recv())['muted'])
                        await controller.send(json.dumps(dict(type='microphone.mute', muted=True)))
                        self.assertTrue(json.loads(await controller.recv())['muted'])
                    self.assertFalse(task.done())
                    self.assertFalse(FakeCapture.instances[-1].opened)
                    errors.assert_not_called()
                finally:
                    task.cancel()
                    await asyncio.gather(task, return_exceptions=True)

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
