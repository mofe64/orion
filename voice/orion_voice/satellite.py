"""Pi-owned capture and Rustpotter, with bounded, authenticated WebSocket sessions."""
from __future__ import annotations

import argparse
import asyncio
from contextlib import suppress
import hmac
import json
from pathlib import Path
import time
import uuid

import numpy as np

from .direction import DirectionEstimator
from .endpoint import EnergyEndpointDetector
from .rustpotter import RustpotterWakeDetector
from .wake import AlsaPcmCapture, DEFAULT_CAPTURE_DEVICE

PROTOCOL = 1
FRAME_BYTES = 640  # 20 ms of mono signed little-endian PCM16 at 16 kHz
MAX_UTTERANCE_BYTES = 18 * 32000


class SatelliteSession:
    """One capture owner, one active turn. Audio is never retained on disk."""
    def __init__(self, wake, direction=None, clock=time.monotonic):
        self.wake = wake
        self.direction = direction or DirectionEstimator()
        self.clock = clock
        self.reset()

    def reset(self):
        self.session_id = None
        self.phase = "listening"
        self.pre_roll = bytearray()
        self.utterance = bytearray()
        self.followup = bytearray()
        self.followup_done = False
        self.endpoint = EnergyEndpointDetector()
        self.followup_endpoint = EnergyEndpointDetector()
        self.expires_at = float("inf")
        self.observation = {"side": "unknown", "confidence": 0.0}
        self.observed_at = float("-inf")
        self.wake.reset()
        self.direction.reset()

    def message(self, kind, **fields):
        return {"type": kind, "sessionId": self.session_id, **fields}

    def accept_stereo(self, pcm: bytes):
        if len(pcm) != FRAME_BYTES * 2:
            raise ValueError("Capture must supply complete 20 ms stereo frames")
        stereo = np.frombuffer(pcm, dtype="<i2").reshape(-1, 2)
        mono = stereo.astype(np.int32).sum(axis=1) // 2
        audio = mono.astype("<i2").tobytes()
        if self.session_id and self.clock() >= self.expires_at:
            event = self.message("session.expired")
            self.reset()
            return [event]
        if self.phase == "listening":
            self.direction.accept(stereo)
            self.pre_roll.extend(audio)
            del self.pre_roll[:-3 * 32000]
            detection = self.wake.process(audio)
            if detection is None:
                return []
            self.session_id = uuid.uuid4().hex
            self.phase = "wake"
            self.expires_at = self.clock() + 120
            self.observation = self.direction.observation()
            self.utterance = bytearray(self.pre_roll)
            self.pre_roll.clear()
            self.endpoint.prime_detected_speech()
            return [self.message("wake.candidate", name=detection.name, score=detection.score,
                                 direction=self.observation)]
        if self.phase in {"wake", "command"}:
            if self.phase == "wake":
                self.direction.accept(stereo)
            self.utterance.extend(audio)
            if self.endpoint.accept(audio):
                return self.finish_utterance()
        elif self.phase == "confirming" and not self.followup_done:
            # Preserve speech spoken while Qwen is confirming a bare wake phrase.
            self.followup.extend(audio)
            self.followup_done = self.followup_endpoint.accept(audio)
        return []

    def finish_utterance(self):
        if self.phase == "wake":
            self.observation = self.direction.observation()
            self.observed_at = self.clock()
        purpose = "wake_and_command" if self.phase == "wake" else "command"
        audio = bytes(self.utterance)
        self.utterance.clear()
        self.phase = "confirming" if purpose == "wake_and_command" else "processing"
        return [self.message("utterance", purpose=purpose, bytes=len(audio)), audio]

    def control(self, message):
        if not self.session_id or message.get("sessionId") != self.session_id:
            raise ValueError("Stale or missing voice session ID")
        kind = message.get("type")
        if kind == "session.finish" and self.phase != "playing":
            raise ValueError("Playback has not started")
        if kind == "session.reject" and self.phase != "confirming":
            raise ValueError("No wake confirmation is pending")
        if kind in {"session.finish", "session.reject", "session.cancel"}:
            self.reset()
            return []
        if kind == "wake.confirmed" and self.phase == "confirming":
            followup = message.get("followup")
            if not isinstance(followup, bool):
                raise ValueError("Wake confirmation requires a followup boolean")
            if followup:
                self.phase = "command"
                self.utterance = self.followup
                self.endpoint = self.followup_endpoint
                self.followup = bytearray()
                if self.followup_done:
                    return self.finish_utterance()
            else:
                self.phase = "processing"
                self.followup.clear()
            return []
        if kind == "session.processing" and self.phase == "processing":
            return []
        if kind == "session.playing" and self.phase == "processing":
            self.phase = "playing"
            self.expires_at = self.clock() + 180
            return []
        raise ValueError("Unexpected voice session transition")


class StereoCapture(AlsaPcmCapture):
    def __init__(self, device=DEFAULT_CAPTURE_DEVICE):
        super().__init__(device)
        self._chunk_bytes = FRAME_BYTES * 2

    def command(self):
        command = super().command()
        command[-1] = "2"
        return command


async def daemon_command(command, socket_path):
    reader, writer = await asyncio.wait_for(asyncio.open_unix_connection(socket_path), 1)
    try:
        writer.write((command + "\n").encode())
        await writer.drain()
        return json.loads(await asyncio.wait_for(reader.readline(), 1))
    finally:
        writer.close()
        await writer.wait_closed()


async def serve(args):
    from websockets.asyncio.server import serve as websocket_serve
    token = args.token_file.read_text().strip()
    if len(token) < 32:
        raise ValueError("Voice token must contain at least 32 characters")
    wake = RustpotterWakeDetector(args.wake_model, args.threshold)
    capture = StereoCapture(args.device)
    session = SatelliteSession(wake, DirectionEstimator(args.mic_spacing, args.channel_sign))
    lock = asyncio.Lock()

    async def character(command):
        # Optional expression must never break a valid speech session.
        with suppress(OSError, ValueError, asyncio.TimeoutError):
            await daemon_command(command, args.daemon_socket)

    async def connection(ws):
        hello = json.loads(await asyncio.wait_for(ws.recv(), 10))
        if (not isinstance(hello, dict) or hello.get("type") != "hello"
                or type(hello.get("protocol")) is not int or hello.get("protocol") != PROTOCOL
                or not isinstance(hello.get("token"), str)
                or not hmac.compare_digest(hello["token"], token)):
            await ws.close(4003, "Invalid listener handshake")
            return
        if lock.locked():
            await ws.close(4009, "Listener already owned")
            return
        async with lock:
            session.reset()
            # Capture is enabled only for the connected, authenticated Studio owner.
            await asyncio.to_thread(capture.open)
            outgoing = asyncio.Queue(maxsize=32)

            async def enqueue(messages):
                for message in messages:
                    outgoing.put_nowait(message)  # Backpressure aborts; never queues stale audio.

            async def send():
                while True:
                    message = await outgoing.get()
                    await asyncio.wait_for(ws.send(message if isinstance(message, bytes) else json.dumps(message)), 5)

            async def listen():
                while True:
                    audio = await asyncio.to_thread(capture.read)
                    events = session.accept_stereo(audio)
                    if any(isinstance(e, dict) and e["type"] == "session.expired" for e in events):
                        await character("character state neutral")
                    await enqueue(events)

            async def controls():
                async for raw in ws:
                    if not isinstance(raw, str):
                        raise ValueError("Listener accepts control messages only")
                    message = json.loads(raw)
                    if not isinstance(message, dict):
                        raise ValueError("Invalid listener control")
                    observation = session.observation.copy()
                    fresh_direction = session.clock() - session.observed_at <= 3.0
                    result = session.control(message)
                    if message["type"] == "wake.confirmed":
                        side, confidence = observation["side"], observation["confidence"]
                        if fresh_direction and side in {"left", "right"} and confidence >= 0.75:
                            await character(f"character attend {side} {confidence:.4f}")
                        await character("character state " + ("listening" if message["followup"] else "thinking"))
                    elif message["type"] == "session.processing":
                        await character("character state thinking")
                    elif message["type"] in {"session.finish", "session.reject", "session.cancel"}:
                        await character("character state neutral")
                    await enqueue(result)

            tasks = []
            try:
                await ws.send(json.dumps({"type": "ready", "protocol": PROTOCOL,
                    "sampleRate": 16000, "channels": 1, "encoding": "pcm_s16le",
                    "wake": {"provider": wake.provider, "model": wake.model_name, "threshold": wake.threshold}}))
                tasks = [asyncio.create_task(job()) for job in (send, listen, controls)]
                done, _ = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
                for task in done:
                    task.result()
            finally:
                # Terminate arecord before waiting for a thread blocked in read().
                capture.close()
                for task in tasks:
                    task.cancel()
                await asyncio.gather(*tasks, return_exceptions=True)
                session.reset()
                await character("character state neutral")
                await ws.close()

    async with websocket_serve(connection, args.host, args.port,
                              max_size=4096, max_queue=16, compression=None):
        await asyncio.Future()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=7448)
    parser.add_argument("--token-file", type=Path, required=True)
    parser.add_argument("--device", default=DEFAULT_CAPTURE_DEVICE)
    parser.add_argument("--wake-model", type=Path, default=Path(__file__).resolve().parents[1] / "models/wake/hey_orion_reference.rpw")
    parser.add_argument("--threshold", type=float, default=0.4)
    parser.add_argument("--mic-spacing", type=float, default=0.0)
    parser.add_argument("--channel-sign", type=int, choices=[-1, 0, 1], default=0)
    parser.add_argument("--daemon-socket", default="/tmp/oriond.sock")
    args = parser.parse_args()
    if not 0 < args.threshold <= 1:
        parser.error("--threshold must be in (0, 1]")
    asyncio.run(serve(args))


if __name__ == "__main__":
    main()
