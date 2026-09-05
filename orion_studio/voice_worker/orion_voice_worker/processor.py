"""Voice processing child owned by the running Studio application."""
from __future__ import annotations

import argparse
import asyncio
from concurrent.futures import ThreadPoolExecutor
from contextlib import suppress
import hmac
import io
import json
import signal
import sys
import threading
import time
import uuid
from urllib.request import Request, urlopen
import wave

from .protocol import event, parse_hello
from .server import handle_connection, load_models


class Gateway:
    def __init__(self, url, token):
        self.url, self.token = url.rstrip('/'), token
        self.executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="voice-upload")

    async def request(self, path, body=None, wav=False, request_id=None):
        headers = {"Authorization": f"Bearer {self.token}",
                   "Content-Type": "audio/wav" if wav else "application/json"}
        if request_id: headers["X-Orion-Voice-Request-ID"] = request_id
        data = body if wav or body is None else json.dumps(body).encode()
        request = Request(self.url + path, data=data, headers=headers)
        def perform():
            with urlopen(request, timeout=5) as response:
                return json.load(response)
        # Uploads and polls must never wait behind model inference.
        return await asyncio.get_running_loop().run_in_executor(self.executor, perform)


def pcm_wav(pcm):
    output = io.BytesIO()
    with wave.open(output, 'wb') as writer:
        writer.setnchannels(1); writer.setsampwidth(2); writer.setframerate(24000)
        writer.writeframes(pcm)
    return output.getvalue()


class ProcessingOwner:
    """Adapts model events to gateway playback and fans status out to observers."""
    def __init__(self, token, gateway, publish):
        self.token, self.gateway, self.publish = token, gateway, publish
        self.controls = asyncio.Queue(maxsize=32)
        self.controls.put_nowait(event("hello", protocol=7, token=token))
        self.session_id = None
        self.metadata = None
        self.run_id = None
        self.request_id = None
        self.upload_id = None
        self.observer = None
        self.ended = asyncio.Event()
        self.closed = False
        self.history = []

    def timing(self, stage):
        self.history.append({"requestId": self.request_id, "stage": stage, "at": time.monotonic()})
        del self.history[:-128]

    async def recv(self): return await self.controls.get()
    def __aiter__(self): return self
    async def __anext__(self):
        if self.closed: raise StopAsyncIteration
        return await self.recv()

    async def send(self, raw):
        if isinstance(raw, bytes):
            metadata, self.metadata = self.metadata, None
            if metadata is None or len(raw) != metadata["samples"] * 2:
                raise ValueError("Invalid synthesis chunk")
            sequence = metadata['sequence']
            if sequence == 0:
                self.request_id = metadata['requestId']
                self.upload_id = f"voice:{self.session_id}" if self.session_id else uuid.uuid4().hex
                self.ended.clear()
                self.timing("first_chunk")
            path = '/api/v2/speech/stream' if sequence == 0 else f'/api/v2/speech/{self.run_id}/chunks/{sequence}'
            accepted = await self.gateway.request(path, pcm_wav(raw), wav=True, request_id=self.upload_id)
            if sequence == 0:
                self.run_id = accepted['run_id']
                self.observer = asyncio.create_task(self.observe(self.run_id, self.request_id))
            return
        message = json.loads(raw)
        kind = message['type']
        if kind == 'wake.candidate': self.session_id = message['sessionId']
        if self.session_id:
            message.setdefault('sessionId', self.session_id)
            raw = json.dumps(message)
        if kind == 'speech.chunk':
            self.metadata = message
            return
        if kind == 'speech.end':
            await self.gateway.request(f'/api/v2/speech/{self.run_id}/end', {"sequence": message['sequence']})
            self.ended.set()
            await self.publish(event('stage.timing', stage='synthesisTotalMs', durationMs=message['synthesisMs']))
            return
        if kind == 'worker.error': await self.cancel_playback()
        await self.publish(raw)

    async def observe(self, run_id, request_id):
        playing = False
        try:
            deadline = time.monotonic() + 150
            while time.monotonic() < deadline:
                status = await self.gateway.request(f'/api/v2/speech/{run_id}')
                if status.get('first_playback_ms') is not None and not playing:
                    playing = True
                    self.timing('playback_start')
                    await self.controls.put(event('playback.started', requestId=request_id))
                    await self.publish(event('speech.started', requestId=request_id, sessionId=self.session_id))
                    await self.publish(event('stage.timing', stage='firstPlaybackMs', durationMs=status['first_playback_ms']))
                if status['state'] == 'completed':
                    await self.ended.wait()
                    self.run_id = None
                    await self.controls.put(event('playback.finished', requestId=request_id))
                    return
                if status['state'] in {'failed', 'cancelled'}:
                    raise RuntimeError(status.get('error') or 'Pi playback stopped')
                await asyncio.sleep(0.1)
            raise TimeoutError('Pi playback timed out')
        except asyncio.CancelledError: raise
        except Exception as error:
            await self.controls.put(event('playback.failed', requestId=request_id, message=str(error)))

    async def cancel_playback(self):
        if self.observer and self.observer is not asyncio.current_task():
            self.observer.cancel()
            await asyncio.gather(self.observer, return_exceptions=True)
        self.observer = None
        if self.run_id is not None:
            run, self.run_id = self.run_id, None
            with suppress(Exception):
                await self.gateway.request('/api/v2/operations', {"operation": "cancel", "kind": "speech", "run_id": run})

    async def close(self, *args):
        self.closed = True
        await self.cancel_playback()


async def microphone(config, muted=None):
    from websockets.asyncio.client import connect
    async with connect(config['pi_url'], open_timeout=5) as pi:
        await pi.send(event('hello', protocol=1, token=config['pi_token'], role='control'))
        status = json.loads(await asyncio.wait_for(pi.recv(), 5))
        if muted is not None:
            await pi.send(event('microphone.mute', muted=muted))
            status = json.loads(await asyncio.wait_for(pi.recv(), 5))
        return {"type": "microphone.status", "muted": status["muted"]}


async def serve(config, parent_closed=None):
    from websockets.asyncio.server import serve as websocket_serve
    from websockets.exceptions import ConnectionClosed
    observers = set()
    ready = None
    replay = []
    async def publish(raw):
        nonlocal ready
        message = json.loads(raw)
        if message['type'] == 'ready':
            ready = raw
            replay.clear()
        else:
            if message['type'] == 'wake.candidate': replay.clear()
            if message['type'] == 'microphone.status' and ready:
                cached = json.loads(ready)
                cached['muted'] = message['muted']
                ready = json.dumps(cached)
            replay.append(raw)
            del replay[:-32]
        # A slow/closed UI has no backpressure on inference or playback.
        for queue in tuple(observers):
            if queue.full(): queue.get_nowait()
            queue.put_nowait(raw)

    async def attach(ws):
        first = await asyncio.wait_for(ws.recv(), 10)
        if not hmac.compare_digest(parse_hello(first).token, config['token']):
            await ws.close(4003, 'Invalid worker token'); return
        queue = asyncio.Queue(maxsize=64)
        observers.add(queue)
        if ready: queue.put_nowait(ready)
        if ready:
            for raw in replay: queue.put_nowait(raw)
        async def send():
            while True: await ws.send(await queue.get())
        async def receive():
            async for raw in ws:
                message = json.loads(raw)
                if message.get('type') == 'stop': return  # Detach only.
                if message.get('type') == 'microphone.mute' and type(message.get('muted')) is bool:
                    await publish(json.dumps(await microphone(config, message['muted'])))
                else: raise ValueError('UI cannot own playback')
        tasks = [asyncio.create_task(send()), asyncio.create_task(receive())]
        try:
            done, _ = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
            for task in done:
                with suppress(ConnectionClosed): task.result()
        finally:
            observers.discard(queue)
            for task in tasks: task.cancel()
            await asyncio.gather(*tasks, return_exceptions=True)

    async def microphone_status():
        previous = None
        while True:
            try:
                status = await microphone(config)
                if status != previous:
                    previous = status
                    await publish(json.dumps(status))
            except Exception:
                pass
            await asyncio.sleep(1)

    async def process():
        models = await asyncio.to_thread(load_models, argparse.Namespace(**config))
        gateway = Gateway(config['gateway_url'], config['pi_token'])
        try:
            while True:
                owner = ProcessingOwner(config['token'], gateway, publish)
                try:
                    await handle_connection(owner, config['token'], models, config['pi_url'], config['pi_token'])
                except asyncio.CancelledError: raise
                except Exception as error:
                    await publish(event('worker.error', code='pi_unavailable', message=str(error), recoverable=True))
                finally:
                    await owner.close()
                await asyncio.sleep(2)
        finally:
            gateway.executor.shutdown(wait=False, cancel_futures=True)
            await asyncio.to_thread(models.agent.close)

    stop = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGTERM, signal.SIGINT): loop.add_signal_handler(sig, stop.set)
    async with websocket_serve(attach, '127.0.0.1', config['port'], max_size=4096, compression=None):
        monitor = asyncio.create_task(microphone_status())
        processor = asyncio.create_task(process())
        stopped = asyncio.create_task(stop.wait())
        try:
            done, _ = await asyncio.wait([processor, stopped] + ([parent_closed] if parent_closed is not None else []), return_when=asyncio.FIRST_COMPLETED)
            for task in done: task.result()
        finally:
            processor.cancel(); stopped.cancel(); monitor.cancel()
            await asyncio.gather(processor, stopped, monitor, return_exceptions=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--mute', choices=['on', 'off', 'status'])
    args = parser.parse_args()
    # Credentials stay in the parent/child pipe, never command arguments or disk.
    config = json.loads(sys.stdin.readline())
    if args.mute:
        print(json.dumps(asyncio.run(microphone(config, {'on': True, 'off': False, 'status': None}[args.mute]))))
        return
    async def run():
        loop = asyncio.get_running_loop()
        loop.set_default_executor(ThreadPoolExecutor(max_workers=1))
        parent_closed = loop.create_future()
        def watch_parent():
            sys.stdin.read()
            def closed():
                if not parent_closed.done(): parent_closed.set_result(None)
            with suppress(RuntimeError): loop.call_soon_threadsafe(closed)
        # This watcher must not keep a failed worker alive while Studio is open.
        threading.Thread(target=watch_parent, name='studio-lifetime', daemon=True).start()
        await serve(config, parent_closed)
    asyncio.run(run())

if __name__ == '__main__': main()
