"""Pi-owned capture and Rustpotter, with bounded, authenticated WebSocket sessions."""
from __future__ import annotations

import argparse
import asyncio
from contextlib import suppress
from collections import deque
import hmac
import json
from pathlib import Path
import time
import uuid

import numpy as np

from .direction import DirectionEstimator
from .endpoint import EndpointConfig, EnergyEndpointDetector, ListeningNoise, pcm16_rms
from .rustpotter import RustpotterWakeDetector
from .capture import AlsaPcmCapture, DEFAULT_CAPTURE_DEVICE

PROTOCOL = 1
FRAME_BYTES = 640  # 20 ms of mono signed little-endian PCM16 at 16 kHz
MAX_UTTERANCE_BYTES = 18 * 32000
CONVERSATION_WINDOW_SECONDS = 5.0
ECHO_GUARD_SECONDS = 0.5
ECHO_QUIET_MS = 300
ECHO_GUARD_LIMIT_SECONDS = 2.0
FOLLOWUP_ONSET_MS = 180


class SatelliteSession:
    """One capture owner, one active turn. Audio is never retained on disk."""
    def __init__(self, wake, direction=None, clock=time.monotonic):
        self.wake = wake
        self.direction = direction or DirectionEstimator(clock=clock)
        self.clock = clock
        self.reset()

    def reset(self):
        self.session_id = None
        self.phase = "listening"
        self.pre_roll = bytearray()
        self.utterance = bytearray()
        self.followup = bytearray()
        self.followup_done = False
        self.noise = ListeningNoise()
        self.endpoint = EnergyEndpointDetector()
        self.followup_endpoint = EnergyEndpointDetector()
        self.expires_at = float("inf")
        self.observation = {"side": "unknown", "confidence": 0.0}
        self.observed_at = float("-inf")
        self.quiet_ms = 0
        self.onset_ms = 0
        self.guard_started = 0.0
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
        if self.phase in {"echo_guard", "conversation"}:
            return self.accept_conversation(audio)
        if self.session_id and self.clock() >= self.expires_at:
            event = self.message("session.expired")
            self.reset()
            return [event]
        if self.phase == "listening":
            self.noise.accept(audio)
            self.direction.accept(stereo)
            self.pre_roll.extend(audio)
            del self.pre_roll[:-3 * 32000]
            detection = self.wake.process(audio)
            if detection is None:
                return []
            self.session_id = uuid.uuid4().hex
            self.phase = "wake"
            config = EndpointConfig(speech_rms=self.noise.threshold())
            self.endpoint = EnergyEndpointDetector(config)
            self.followup_endpoint = EnergyEndpointDetector(config)
            self.expires_at = self.clock() + 120
            self.update_direction()
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

    def accept_conversation(self, audio):
        now = self.clock()
        if self.phase == "echo_guard":
            # Discard playback tail and require a quiet baseline before accepting onset.
            if now - self.guard_started >= ECHO_GUARD_SECONDS:
                self.quiet_ms = self.quiet_ms + 20 if pcm16_rms(audio) < self.endpoint.config.speech_rms else 0
                if self.quiet_ms >= ECHO_QUIET_MS:
                    self.phase = "conversation"
                    self.expires_at = now + CONVERSATION_WINDOW_SECONDS
                    return [self.message("conversation.ready", durationMs=round(CONVERSATION_WINDOW_SECONDS * 1000))]
            if now - self.guard_started >= ECHO_GUARD_LIMIT_SECONDS:
                return self.close_conversation("echo_guard_timeout")
            return []
        if now >= self.expires_at and self.onset_ms == 0:
            return self.close_conversation("timeout")
        self.pre_roll.extend(audio)
        del self.pre_roll[:-int(0.3 * 32000)]
        self.onset_ms = self.onset_ms + 20 if pcm16_rms(audio) >= self.endpoint.config.speech_rms else 0
        if self.onset_ms < FOLLOWUP_ONSET_MS:
            return []
        previous = self.session_id
        self.session_id = uuid.uuid4().hex
        self.phase = "command"
        self.expires_at = now + 120
        self.utterance = bytearray(self.pre_roll)
        self.pre_roll.clear()
        self.endpoint = EnergyEndpointDetector(self.endpoint.config)
        self.endpoint.prime_detected_speech()
        return [self.message("command.candidate", previousSessionId=previous)]

    def close_conversation(self, reason):
        message = self.message("conversation.closed", reason=reason)
        self.reset()
        return [message]

    def update_direction(self):
        observation = self.direction.observation()
        observed_at = observation.pop("observed_at")
        self.observed_at = observed_at if observed_at is not None else float("-inf")
        self.observation = observation

    def direction_is_fresh(self):
        return 0 <= self.clock() - self.observed_at < 3.0

    def finish_utterance(self):
        if self.phase == "wake":
            self.update_direction()
        purpose = "wake_and_command" if self.phase == "wake" else "command"
        print(json.dumps({"event": "voice.endpoint", "session_id": self.session_id,
                          "purpose": purpose, "reason": self.endpoint.end_reason,
                          "threshold_rms": round(self.endpoint.config.speech_rms, 1),
                          "capture_ms": self.endpoint.capture_ms}), flush=True)
        audio = bytes(self.utterance)
        self.utterance.clear()
        self.phase = "confirming" if purpose == "wake_and_command" else "processing"
        return [self.message("utterance", purpose=purpose, bytes=len(audio), captureMs=self.endpoint.capture_ms), audio]

    def control(self, message):
        if not self.session_id or message.get("sessionId") != self.session_id:
            raise ValueError("Stale or missing voice session ID")
        kind = message.get("type")
        if kind == "session.finish" and self.phase != "playing":
            raise ValueError("Playback has not started")
        if kind == "session.reject" and self.phase != "confirming":
            raise ValueError("No wake confirmation is pending")
        if kind == "session.finish" and message.get("conversationWindow") is True:
            self.phase = "echo_guard"
            self.guard_started = self.clock()
            self.quiet_ms = self.onset_ms = 0
            self.pre_roll.clear()
            self.followup.clear()
            self.utterance.clear()
            return []
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
        if kind == "session.processing" and self.phase in {"processing", "playing"}:
            self.phase = "processing"
            self.expires_at = self.clock() + 180
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
    from websockets.exceptions import ConnectionClosed
    token = args.token_file.read_text().strip()
    if len(token) < 32:
        raise ValueError("Voice token must contain at least 32 characters")
    wake = RustpotterWakeDetector(args.wake_model, args.threshold)
    capture = StereoCapture(args.device)
    session = SatelliteSession(wake, DirectionEstimator(args.mic_spacing, args.channel_sign))
    lock = asyncio.Lock()
    capture_gate = asyncio.Lock()
    mute_file = getattr(args, "mute_file", args.token_file.with_name("microphone.json"))
    muted = json.loads(mute_file.read_text())["muted"] if mute_file.exists() else False
    if type(muted) is not bool:
        raise ValueError("Invalid saved microphone preference")
    owner = None
    outgoing = None
    generation = 0
    timing_history = deque(maxlen=128)
    changed = asyncio.Event()
    feedback = asyncio.Queue(maxsize=64)

    def expression(kind, session_id=None):
        identity = session_id or session.session_id
        if identity:
            try:
                feedback.put_nowait(f"voice {identity} {kind}")
            except asyncio.QueueFull:
                # The runtime's own lease bounds feedback if its socket is unavailable.
                pass

    def cancel_turn():
        nonlocal generation
        expression("cancel")
        generation += 1
        session.reset()
        if outgoing is not None:
            while not outgoing.empty(): outgoing.get_nowait()

    async def character():
        while True:
            command = await feedback.get()
            with suppress(OSError, ValueError, asyncio.TimeoutError):
                await daemon_command(command, args.daemon_socket)

    async def deliver(messages):
        nonlocal outgoing
        for message in messages:
            if isinstance(message, dict):
                kind = message["type"]
                if kind in {"wake.candidate", "utterance"}:
                    timing_history.append({"sessionId": message["sessionId"], "event": kind, "at": time.monotonic()})
                if kind == "wake.candidate": expression("wake")
                elif kind == "command.candidate":
                    expression("finish", message["previousSessionId"])
                    expression("continue", message["sessionId"])
                elif kind == "conversation.ready": expression("window", message["sessionId"])
                elif kind == "conversation.closed": expression("finish", message["sessionId"])
                elif kind == "utterance": expression("endpoint" if owner is not None else "unavailable")
                elif kind == "session.expired": expression("cancel", message["sessionId"])
            if outgoing is not None:
                try:
                    outgoing.put_nowait(message)
                except asyncio.QueueFull:
                    await owner.close(4012, "Processing connection stalled")
                    cancel_turn()
                    return
        if owner is None and any(isinstance(m, dict) and m["type"] == "utterance" for m in messages):
            session.reset()

    async def listen():
        nonlocal generation
        opened = False
        try:
            while True:
                if muted:
                    if opened:
                        capture.close()
                        opened = False
                    changed.clear()
                    await changed.wait()
                    continue
                if not opened:
                    async with capture_gate:
                        if muted: continue
                        opening = asyncio.create_task(asyncio.to_thread(capture.open))
                        try:
                            await asyncio.shield(opening)
                        except asyncio.CancelledError:
                            # Cancelling to_thread cannot cancel resource creation.
                            # Retire the opener before allowing service shutdown.
                            await opening
                            capture.close()
                            raise
                        opened = True
                    if muted: continue
                epoch = generation
                try:
                    audio = await asyncio.to_thread(capture.read)
                except Exception:
                    capture.close()
                    opened = False
                    cancel_turn()
                    await asyncio.sleep(0.25)
                    continue
                if epoch == generation and not muted:
                    await deliver(session.accept_stereo(audio))
        finally:
            capture.close()

    async def set_muted(value):
        nonlocal muted
        if type(value) is not bool: raise ValueError("Mute requires a boolean")
        # Write before acknowledging, so success means the preference is durable.
        mute_file.parent.mkdir(parents=True, exist_ok=True)
        temporary = mute_file.with_suffix(".tmp")
        temporary.write_text(json.dumps({"muted": value}))
        temporary.chmod(0o600)
        temporary.replace(mute_file)
        if muted == value: return
        interrupted = session.session_id
        muted = value
        cancel_turn()
        if muted:
            async with capture_gate:
                capture.close()
        changed.set()
        if interrupted and outgoing is not None:
            outgoing.put_nowait({"type": "session.expired", "sessionId": interrupted})

    async def connection(ws):
        nonlocal owner, outgoing
        hello = json.loads(await asyncio.wait_for(ws.recv(), 10))
        if (not isinstance(hello, dict) or hello.get("type") != "hello"
                or type(hello.get("protocol")) is not int or hello.get("protocol") != PROTOCOL
                or not isinstance(hello.get("token"), str)
                or not hmac.compare_digest(hello["token"], token)):
            await ws.close(4003, "Invalid listener handshake")
            return
        if hello.get("role") == "control":
            # Status clients don't own capture; their disconnect ends only
            # this control request, including when they disappear abruptly.
            with suppress(ConnectionClosed):
                await ws.send(json.dumps({"type": "microphone.status", "muted": muted, "timingHistory": list(timing_history)}))
                async for raw in ws:
                    message = json.loads(raw)
                    if message.get("type") != "microphone.mute": raise ValueError("Invalid microphone control")
                    await set_muted(message.get("muted"))
                    await ws.send(json.dumps({"type": "microphone.status", "muted": muted, "timingHistory": list(timing_history)}))
            return
        if lock.locked():
            await ws.close(4009, "Listener already owned")
            return
        async with lock:
            cancel_turn()
            owner = ws
            outgoing = asyncio.Queue(maxsize=32)
            queue = outgoing

            async def send():
                while True:
                    message = await queue.get()
                    await asyncio.wait_for(ws.send(message if isinstance(message, bytes) else json.dumps(message)), 5)

            async def controls():
                async for raw in ws:
                    if not isinstance(raw, str): raise ValueError("Listener accepts control messages only")
                    message = json.loads(raw)
                    if not isinstance(message, dict): raise ValueError("Invalid listener control")
                    if message.get("type") == "microphone.mute":
                        await set_muted(message.get("muted"))
                        continue
                    # Late completion from a cancelled turn has no authority.
                    if message.get("sessionId") != session.session_id: continue
                    identity = session.session_id
                    observation = session.observation.copy()
                    fresh_direction = session.direction_is_fresh()
                    result = session.control(message)
                    if message["type"] == "session.processing":
                        expression("processing", identity)
                    elif message["type"] == "wake.confirmed":
                        side, confidence = observation["side"], observation["confidence"]
                        if fresh_direction and side in {"left", "right"} and confidence >= 0.75:
                            expression(f"attend_{side}", identity)
                        if message["followup"]: expression("followup", identity)
                    elif message["type"] in {"session.finish", "session.reject", "session.cancel"}:
                        expression("guard" if message["type"] == "session.finish" and session.phase == "echo_guard"
                                   else message["type"].split(".")[1], identity)
                    await deliver(result)

            tasks = []
            try:
                await ws.send(json.dumps({"type": "ready", "protocol": PROTOCOL,
                    "sampleRate": 16000, "channels": 1, "encoding": "pcm_s16le", "muted": muted,
                    "conversationWindow": True, "toolFeedback": True,
                    "wake": {"provider": wake.provider, "model": wake.model_name, "threshold": wake.threshold}}))
                tasks = [asyncio.create_task(job()) for job in (send, controls)]
                done, _ = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
                for task in done: task.result()
            finally:
                owner = None
                outgoing = None
                if session.phase not in {"wake", "command"}:
                    if session.session_id:
                        expression("unavailable")
                        session.reset()
                    else:
                        cancel_turn()
                for task in tasks: task.cancel()
                await asyncio.gather(*tasks, return_exceptions=True)
                await ws.close()

    tasks = [asyncio.create_task(listen()), asyncio.create_task(character())]
    try:
        async with websocket_serve(connection, args.host, args.port,
                                  max_size=4096, max_queue=16, compression=None, ping_interval=5, ping_timeout=5):
            await asyncio.gather(*tasks)
    finally:
        capture.close()
        for task in tasks: task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
        if session.session_id:
            with suppress(OSError, ValueError, asyncio.TimeoutError):
                await daemon_command(f"voice {session.session_id} cancel", args.daemon_socket)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=7448)
    parser.add_argument("--token-file", type=Path, required=True)
    parser.add_argument("--mute-file", type=Path, default=Path.home() / ".config/orion/microphone.json")
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
