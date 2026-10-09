"""Pi-owned capture, Rustpotter and phrase verification, with bounded, authenticated WebSocket sessions."""
from __future__ import annotations

import argparse
import asyncio
from contextlib import suppress
from collections import deque
import hmac
import ipaddress
import json
import os
from pathlib import Path
import time
import uuid

import numpy as np

from .direction import DirectionEstimator, XvfDirectionEstimator
from .endpoint import EndpointConfig, EnergyEndpointDetector, ListeningNoise, pcm16_rms
from .rustpotter import RustpotterWakeDetector
from .verifier import AcousticVerifier
from .capture import AlsaPcmCapture, DEFAULT_CAPTURE_DEVICE
from .wake_debug import CaptureReadStats, WakeDebugRecorder

PROTOCOL = 1
FRAME_BYTES = 640  # 20 ms of mono signed little-endian PCM16 at 16 kHz
MAX_UTTERANCE_BYTES = 33 * 32000
CONVERSATION_WINDOW_SECONDS = 5.0
ECHO_GUARD_SECONDS = 0.5
ECHO_QUIET_MS = 300
ECHO_GUARD_LIMIT_SECONDS = 2.0
FOLLOWUP_ONSET_MS = 180
WAKE_PREFIX_BYTES = 2 * 32000
WAKE_PREFIX_TAIL_MS = 200


class SatelliteSession:
    """One capture owner, one active turn. Disk audio requires opt-in diagnostics."""
    def __init__(self, wake, direction=None, clock=time.monotonic, endpoint_factory=EnergyEndpointDetector,
                 early_wake=False, verifier=None, barge_in=False, wake_debug=None):
        self.wake = wake
        # The verifier hears every captured frame, across sessions, so its
        # score history already covers the phrase when Rustpotter fires.
        self.verifier = verifier
        self.wake_debug = wake_debug
        self.barge_in = barge_in
        self.direction = direction or DirectionEstimator(clock=clock)
        self.clock = clock
        self.endpoint_factory = endpoint_factory
        self.alarm_active = False
        self.alarm_guard_until = 0.0
        self.early_wake = early_wake
        self.reset()

    def reset(self, preserve_listening=False):
        self.session_id = None
        self.phase = "listening"
        if not preserve_listening:
            self.pre_roll = bytearray()
            self.noise = ListeningNoise()
        self.utterance = bytearray()
        self.followup = bytearray()
        self.followup_done = False
        self.prefix = bytearray()
        self.prefix_pending = False
        self.prefix_sent = False
        self.acoustic_pending = False
        self.acoustic_verdict = None
        self.verifier_candidate = None
        self.barge_candidate = None
        self.barge_in_session = False
        self.pending_utterance = []
        self.endpoint = self.endpoint_factory(EndpointConfig())
        self.followup_endpoint = self.endpoint_factory(EndpointConfig())
        self.activity = self.endpoint_factory(EndpointConfig())
        self.expires_at = float("inf")
        self.observation = {"side": "unknown", "confidence": 0.0}
        self.observed_at = float("-inf")
        self.quiet_ms = 0
        self.onset_ms = 0
        self.guard_started = 0.0
        if not preserve_listening:
            self.wake.reset()
            self.direction.reset()
        if self.verifier is not None:
            self.verifier.cancel()
        if self.wake_debug is not None:
            self.wake_debug.cancel()

    def set_alarm(self, active):
        if active == self.alarm_active: return None
        interrupted = self.session_id
        self.reset()
        self.alarm_active = active
        self.alarm_guard_until = 0.0 if active else self.clock() + ECHO_GUARD_SECONDS
        return interrupted

    def message(self, kind, **fields):
        return {"type": kind, "sessionId": self.session_id, **fields}

    def verifier_active(self):
        """Whether verification is fast enough to authorize playback barge-in."""
        return self.verifier is not None and self.verifier.healthy

    def restart_verifier(self):
        """Discard score history after a capture gap; the stream is no longer continuous."""
        if self.verifier is not None:
            self.verifier.reset()
        if self.wake_debug is not None:
            self.wake_debug.capture_opened()

    def accept_stereo(self, pcm: bytes):
        if len(pcm) != FRAME_BYTES * 2:
            raise ValueError("Capture must supply complete 20 ms stereo frames")
        stereo = np.frombuffer(pcm, dtype="<i2").reshape(-1, 2)
        mono = stereo.astype(np.int32).sum(axis=1) // 2
        if self.verifier is not None:
            chunks = self.verifier.feed(mono.astype(np.int16))
            if self.wake_debug is not None:
                self.wake_debug.accept(mono.astype("<i2").tobytes(), chunks, self.verifier.position)
        messages = self._accept_frame(stereo, mono.astype("<i2").tobytes())
        if self.acoustic_pending or self.barge_candidate is not None:
            messages.extend(self.acoustic_decision())
        return messages

    def acoustic_decision(self):
        if self.barge_candidate is not None and not self.verifier_active():
            self.barge_candidate = None
            self.verifier.cancel()
            return []
        verdict = self.verifier.verdict()
        if verdict is None:
            return []
        if self.wake_debug is not None:
            self.wake_debug.verdict(verdict, self.verifier)
        interrupted = []
        if self.barge_candidate is not None:
            detection, self.barge_candidate = self.barge_candidate, None
            if not verdict.accepted:
                return []
            interrupted = [self.message("session.interrupted", reason="barge_in")]
            pre_roll, noise = self.pre_roll, self.noise
            candidate = self.verifier_candidate
            self.reset()
            self.pre_roll, self.noise = pre_roll, noise
            self.barge_in_session = True
            interrupted.extend(self.start_wake(detection, already_verified=True))
            self.verifier_candidate = candidate
        self.acoustic_pending = False
        self.acoustic_verdict = verdict.accepted
        latency_ms = round((verdict.decided_sample - self.verifier_candidate) / 16)
        message = self.message("wake.verified", accepted=verdict.accepted, source="acoustic",
                               score=None if verdict.score is None else round(verdict.score, 4),
                               verifierMs=latency_ms)
        print(json.dumps({"event": "voice.wake_verifier", "session_id": self.session_id,
                          "accepted": verdict.accepted, "score": message["score"],
                          "verifier_ms": latency_ms}), flush=True)
        if not verdict.accepted:
            # Retire only this candidate. Capture, detector state and pre-roll
            # stay continuous so another wake can start immediately.
            self.reset(preserve_listening=True)
            return [message]
        self.pre_roll.clear()
        # An endpoint that arrived during verification was held back, like a
        # pending ASR prefix; release it after the verdict so ordering holds.
        pending, self.pending_utterance = self.pending_utterance, []
        return [*interrupted, message, *pending]

    def start_wake(self, detection, already_verified=False):
        candidate_phase = self.phase
        self.session_id = uuid.uuid4().hex
        self.phase = "wake"
        config = EndpointConfig(speech_rms=self.noise.threshold())
        self.endpoint = self.endpoint_factory(config)
        self.followup_endpoint = self.endpoint_factory(config)
        self.expires_at = self.clock() + 120
        if not self.barge_in_session:
            self.update_direction()
        self.utterance = bytearray(self.pre_roll)
        verifying = already_verified or self.verifier is not None
        if verifying and not already_verified:
            self.verifier.begin()
            self.verifier_candidate = self.verifier.candidate
            self.acoustic_pending = True
            if self.wake_debug is not None:
                self.wake_debug.begin(self.session_id, candidate_phase, detection.score, self.verifier_candidate)
        elif not verifying and self.early_wake:
            self.prefix = bytearray(self.pre_roll[-WAKE_PREFIX_BYTES:])
            self.prefix_pending = True
        if not self.acoustic_pending:
            self.pre_roll.clear()
        self.endpoint.prime_detected_speech()
        return [self.message("wake.candidate", name=detection.name, score=detection.score,
                             direction=self.observation, acousticVerification=verifying)]

    def _accept_frame(self, stereo, audio):
        if self.alarm_active:
            # Alarm dismissal is local and never waits for ASR or the agent.
            if self.wake.process(audio) is not None:
                return [{"type": "alarm.dismiss"}]
            return []
        if self.clock() < self.alarm_guard_until:
            return []
        if self.phase in {"echo_guard", "conversation"}:
            return self.accept_conversation(audio)
        if self.session_id and self.clock() >= self.expires_at:
            event = self.message("session.expired")
            self.reset()
            return [event]
        if self.phase == "playing":
            if self.barge_in and self.verifier_active():
                self.pre_roll.extend(audio)
                del self.pre_roll[:-3 * 32000]
                if self.barge_candidate is None:
                    detection = self.wake.process(audio)
                    if detection is not None:
                        self.barge_candidate = detection
                        self.verifier.begin()
                        self.verifier_candidate = self.verifier.candidate
                        if self.wake_debug is not None:
                            self.wake_debug.begin(self.session_id, self.phase, detection.score, self.verifier_candidate)
            return []
        if self.acoustic_pending:
            # Keep the listening stream warm while holding one candidate's
            # endpoint. Extra detections cannot overlap the pending verdict.
            self.pre_roll.extend(audio)
            del self.pre_roll[:-3 * 32000]
            self.wake.process(audio)
        if self.phase == "listening":
            self.noise.accept(audio)
            self.direction.accept(stereo)
            self.pre_roll.extend(audio)
            del self.pre_roll[:-3 * 32000]
            detection = self.wake.process(audio)
            if detection is None:
                return []
            return self.start_wake(detection)
        if self.phase in {"wake", "command"}:
            if self.phase == "wake" and not self.barge_in_session:
                self.direction.accept(stereo)
            self.utterance.extend(audio)
            messages = []
            if self.phase == "wake" and self.prefix_pending and not self.prefix_sent:
                self.prefix.extend(audio)
                if self.endpoint.capture_ms + 20 >= WAKE_PREFIX_TAIL_MS:
                    prefix = bytes(self.prefix)
                    self.prefix.clear()
                    self.prefix_sent = True
                    messages = [self.message("utterance", purpose="wake_prefix", bytes=len(prefix),
                                             captureMs=round(len(prefix) / 32)), prefix]
            if self.endpoint.accept(audio):
                messages.extend(self.finish_utterance())
            return messages
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
                self.quiet_ms = self.quiet_ms + 20 if not self.activity.is_speech(audio) else 0
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
        self.onset_ms = self.onset_ms + 20 if self.activity.is_speech(audio) else 0
        if self.onset_ms < FOLLOWUP_ONSET_MS:
            return []
        previous = self.session_id
        self.session_id = uuid.uuid4().hex
        self.phase = "command"
        self.expires_at = now + 120
        self.utterance = bytearray(self.pre_roll)
        self.pre_roll.clear()
        self.endpoint = self.endpoint_factory(self.endpoint.config)
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
        if self.phase == "wake" and not self.barge_in_session:
            self.update_direction()
        purpose = "wake_and_command" if self.phase == "wake" else "command"
        print(json.dumps({"event": "voice.endpoint", "session_id": self.session_id,
                          "purpose": purpose, "reason": self.endpoint.end_reason,
                          "threshold_rms": round(self.endpoint.config.speech_rms, 1),
                          "capture_ms": self.endpoint.capture_ms}), flush=True)
        audio = bytes(self.utterance)
        self.utterance.clear()
        self.phase = "confirming" if purpose == "wake_and_command" else "processing"
        messages = [self.message("utterance", purpose=purpose, bytes=len(audio), captureMs=self.endpoint.capture_ms,
                                 endReason=self.endpoint.end_reason), audio]
        if purpose == "wake_and_command" and (self.prefix_pending or self.acoustic_pending):
            # Keep capturing follow-up speech while the short ASR pass finishes.
            # Only one ASR request can own this session at a time.
            self.pending_utterance = messages
            return []
        return messages

    def control(self, message):
        if not self.session_id or message.get("sessionId") != self.session_id:
            raise ValueError("Stale or missing voice session ID")
        kind = message.get("type")
        if kind == "wake.verified":
            if (not self.prefix_pending or not self.prefix_sent
                    or self.phase not in {"wake", "confirming"}
                    or type(message.get("accepted")) is not bool):
                raise ValueError("No wake prefix verification is pending")
            self.prefix_pending = False
            pending, self.pending_utterance = self.pending_utterance, []
            return pending
        if kind == "session.finish" and self.phase != "playing":
            raise ValueError("Playback has not started")
        if kind == "session.reject" and self.phase != "confirming":
            raise ValueError("No wake confirmation is pending")
        if kind == "session.finish" and message.get("conversationWindow") is True:
            self.barge_candidate = None
            if self.verifier is not None:
                self.verifier.cancel()
            self.phase = "echo_guard"
            self.guard_started = self.clock()
            self.quiet_ms = self.onset_ms = 0
            self.activity = self.endpoint_factory(self.endpoint.config)
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
        if kind == "session.keepalive" and self.phase in {"processing", "playing"}:
            if self.clock() >= self.expires_at:
                raise ValueError("Expired voice session ID")
            self.expires_at = self.clock() + 180
            return []
        if kind == "session.processing" and self.phase in {"processing", "playing"}:
            self.barge_candidate = None
            if self.verifier is not None: self.verifier.cancel()
            self.pre_roll.clear()
            self.phase = "processing"
            self.expires_at = self.clock() + 180
            return []
        if kind == "session.playing" and self.phase in {"processing", "playing"}:
            if self.phase != "playing":
                self.pre_roll.clear()
                self.wake.reset()
            self.phase = "playing"
            self.expires_at = self.clock() + 180
            return []
        raise ValueError("Unexpected voice session transition")


class StereoCapture(AlsaPcmCapture):
    def __init__(self, device=DEFAULT_CAPTURE_DEVICE, *, hardware="v1", capture_channels=2, processed_channel=0):
        if capture_channels not in (2, 6) or not 0 <= processed_channel < capture_channels:
            raise ValueError("Capture needs 2 or 6 channels and a valid processed channel")
        if hardware == "v1" and capture_channels != 2:
            raise ValueError("V1 requires the two raw HAT microphone channels")
        super().__init__(device, hardware=hardware)
        self.capture_channels = capture_channels
        self.processed_channel = processed_channel
        self._chunk_bytes = FRAME_BYTES * capture_channels

    def command(self):
        command = super().command()
        command[-1] = str(self.capture_channels)
        return command

    def read(self):
        pcm = super().read()
        if self._hardware == "v1":
            return pcm
        # Feed the existing mono wake/ASR contract from one processed USB
        # channel. Replication preserves framing without mixing in raw mics.
        samples = np.frombuffer(pcm, dtype="<i2").reshape(-1, self.capture_channels)
        return np.repeat(samples[:, self.processed_channel], 2).astype("<i2").tobytes()


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
    verifier = AcousticVerifier(args.verifier_dir) if getattr(args, "verifier_dir", None) else None
    hardware = getattr(args, "hardware", "v1")
    capture = (StereoCapture(args.device) if hardware == "v1" else
        StereoCapture(args.device, hardware=hardware,
            capture_channels=getattr(args, "capture_channels", 2), processed_channel=getattr(args, "processed_channel", 0)))
    endpoint_factory = EnergyEndpointDetector
    if getattr(args, "vad_model", None):
        from .vad import SileroModel
        endpoint_factory = SileroModel(args.vad_model).endpoint
    direction = DirectionEstimator(args.mic_spacing if hardware == "v1" else 0, args.channel_sign if hardware == "v1" else 0)
    if hardware == "v2" and getattr(args, "xvf_direction", 0):
        direction = XvfDirectionEstimator(getattr(args, "xvf_azimuth_offset_deg", None),
            getattr(args, "xvf_azimuth_sign", None), getattr(args, "xvf_front_half_width_deg", 30),
            getattr(args, "xvf_min_energy", 0))
    session = SatelliteSession(wake, direction,
                               endpoint_factory=endpoint_factory, verifier=verifier, barge_in=hardware == "v2")
    lock = asyncio.Lock()
    capture_gate = asyncio.Lock()
    capture_ready = asyncio.Event()
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
    background = set()
    wake_debug = WakeDebugRecorder.from_environment()
    session.wake_debug = wake_debug

    def expression(kind, session_id=None):
        identity = session_id or session.session_id
        if identity:
            try:
                feedback.put_nowait((f"voice {identity} {kind}", None, time.monotonic()))
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
            command, receipt, queued_at = await feedback.get()
            if receipt is not None and receipt.cancelled(): continue
            fields = command.split()
            if len(fields) == 4 and fields[2] in {'attend_left', 'attend_right'}:
                fields[3] = str(float(fields[3]) + (time.monotonic() - queued_at) * 1000)
                command = ' '.join(fields)
            try:
                result = await daemon_command(command, args.daemon_socket)
                if receipt is not None and not receipt.done(): receipt.set_result(result)
            except (OSError, ValueError, asyncio.TimeoutError) as error:
                if receipt is not None and not receipt.done(): receipt.set_exception(error)

    async def confirm_activity(identity):
        # Use the same FIFO as candidate/endpoint feedback. Confirmation must
        # reach oriond even when direction is unknown; a lost notification must
        # fail the turn instead of silently leaving the body asleep.
        receipt = asyncio.get_running_loop().create_future()
        await feedback.put((f"voice {identity} confirmed", receipt, time.monotonic()))
        result = await asyncio.wait_for(receipt, 5)
        if result.get("ok") is not True:
            raise ValueError("Runtime did not accept wake confirmation")

    async def confirmed_feedback(identity, observation, observed_at, followup=False, skip_attention=False):
        await confirm_activity(identity)
        side, confidence = observation["side"], observation["confidence"]
        direction_age = session.clock() - observed_at
        if not skip_attention and 0 <= direction_age < 3.0 and side in {"left", "right"} and confidence >= 0.75:
            expression(f"attend_{side} {direction_age * 1000:.3f}", identity)
        if followup: expression("followup", identity)

    async def acoustic_confirmation(identity, observation, observed_at, skip_attention=False):
        # Runs beside capture so waiting for oriond never delays microphone reads.
        try:
            await confirmed_feedback(identity, observation, observed_at, skip_attention=skip_attention)
        except (OSError, ValueError, asyncio.TimeoutError):
            if session.session_id == identity:
                cancel_turn()
                if outgoing is not None:
                    with suppress(asyncio.QueueFull):
                        outgoing.put_nowait({"type": "session.expired", "sessionId": identity})

    async def deliver(messages):
        nonlocal outgoing
        for message in messages:
            if isinstance(message, dict):
                kind = message["type"]
                if kind == "alarm.dismiss":
                    try:
                        result = await daemon_command('routines {"action":"stop"}', args.daemon_socket)
                        if result.get("ok") is True:
                            session.set_alarm(False)
                            # Discard the speaker tail before normal wake detection resumes.
                            session.wake.reset()
                    except (OSError, ValueError, asyncio.TimeoutError):
                        pass
                    continue
                if kind in {"wake.candidate", "wake.verified", "utterance"}:
                    timing_history.append({"sessionId": message["sessionId"], "event": kind, "at": time.monotonic()})
                if kind == "wake.candidate":
                    # With acoustic verification, the chime and light pulse wait for acceptance.
                    if not message.get("acousticVerification"): expression("wake")
                elif kind == "wake.verified" and message["accepted"]:
                    # oriond opens its voice session on "wake" (chime and pulse) and
                    # accepts confirmation while listening only after "verify".
                    expression("wake")
                    expression("verify")
                    background.add(asyncio.create_task(acoustic_confirmation(
                        message["sessionId"], session.observation.copy(), session.observed_at,
                        session.barge_in_session)))
                    for task in list(background):
                        if task.done(): background.discard(task)
                elif kind == "wake.verified":
                    # The session has already returned to listening; use the
                    # rejected ID rather than the current (possibly empty) ID.
                    expression("cancel", message["sessionId"])
                elif kind == "command.candidate":
                    expression("finish", message["previousSessionId"])
                    expression("continue", message["sessionId"])
                elif kind == "conversation.ready": expression("window", message["sessionId"])
                elif kind == "conversation.closed": expression("finish", message["sessionId"])
                elif kind == "utterance":
                    expression(("verify" if message["purpose"] == "wake_prefix" else "endpoint")
                               if owner is not None else "unavailable")
                elif kind == "session.expired": expression("cancel", message["sessionId"])
                elif kind == "session.interrupted": expression("cancel", message["sessionId"])
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
        read_stats = CaptureReadStats()
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
                        session.restart_verifier()
                        capture_ready.set()
                    if muted: continue
                epoch = generation
                read_started = time.monotonic()
                try:
                    try:
                        audio = await asyncio.to_thread(capture.read)
                    finally:
                        read_stats.accept(time.monotonic() - read_started)
                except Exception:
                    capture_ready.clear()
                    capture.close()
                    opened = False
                    cancel_turn()
                    await asyncio.sleep(0.25)
                    continue
                if epoch == generation and not muted:
                    await deliver(session.accept_stereo(audio))
        finally:
            capture_ready.clear()
            capture.close()

    async def alarms():
        while True:
            try:
                result = await daemon_command("routines status", args.daemon_socket)
                active = result.get("routines", {}).get("ringing")
                if type(active) is bool and active != session.alarm_active:
                    interrupted = session.session_id
                    cancel_turn()
                    session.set_alarm(active)
                    if interrupted and outgoing is not None:
                        await deliver([{"type":"session.interrupted", "sessionId":interrupted, "reason":"alarm"}])
            except (OSError, ValueError, asyncio.TimeoutError):
                pass
            await asyncio.sleep(0.2)

    async def set_muted(value):
        nonlocal muted
        if type(value) is not bool: raise ValueError("Mute requires a boolean")
        # Write before acknowledging, so success means the preference is durable.
        mute_file.parent.mkdir(parents=True, exist_ok=True)
        temporary = mute_file.with_suffix(".tmp")
        temporary.write_text(json.dumps({"muted": value}))
        temporary.chmod(0o600)
        temporary.replace(mute_file)
        if muted == value:
            if not muted: await asyncio.wait_for(capture_ready.wait(), 10)
            return
        interrupted = session.session_id
        muted = value
        cancel_turn()
        if muted:
            async with capture_gate:
                capture.close()
                capture_ready.clear()
        changed.set()
        if interrupted and outgoing is not None:
            outgoing.put_nowait({"type": "session.expired", "sessionId": interrupted})
        if not muted:
            await asyncio.wait_for(capture_ready.wait(), 10)

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
        if getattr(args, "local_processor", False) and not ipaddress.ip_address(ws.remote_address[0]).is_loopback:
            await ws.close(4003, "Orion uses its onboard coordinator")
            return
        if lock.locked():
            await ws.close(4009, "Listener already owned")
            return
        async with lock:
            cancel_turn()
            session.early_wake = hello.get("wakePrefix") is True
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
                    observed_at = session.observed_at
                    skip_attention = session.barge_in_session
                    result = session.control(message)
                    if message["type"] == "session.playing":
                        expression("playing")
                    if message["type"] == "session.keepalive":
                        expression("keepalive", identity)
                    elif message["type"] == "session.processing":
                        expression("processing", identity)
                    elif message["type"] == "wake.confirmed" or (
                            message["type"] == "wake.verified" and message["accepted"]):
                        await confirmed_feedback(identity, observation, observed_at, message.get("followup"), skip_attention)
                    elif message["type"] in {"session.finish", "session.reject", "session.cancel"}:
                        expression("guard" if message["type"] == "session.finish" and session.phase == "echo_guard"
                                   else message["type"].split(".")[1], identity)
                    await deliver(result)

            tasks = []
            try:
                if not muted:
                    await asyncio.wait_for(capture_ready.wait(), 10)
                await ws.send(json.dumps({"type": "ready", "protocol": PROTOCOL,
                    "sampleRate": 16000, "channels": 1, "encoding": "pcm_s16le", "muted": muted,
                    "conversationWindow": True, "toolFeedback": True,
                    "wakePrefix": session.early_wake,
                    "maxUtteranceBytes": MAX_UTTERANCE_BYTES,
                    "vad": "silero" if getattr(args, "vad_model", None) else "energy",
                    "wake": {"provider": wake.provider, "model": wake.model_name, "threshold": wake.threshold,
                             "verifier": verifier.describe() if verifier is not None else None}}))
                tasks = [asyncio.create_task(job()) for job in (send, controls)]
                done, _ = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
                for task in done: task.result()
            except ConnectionClosed:
                # Owner restarts are normal; the finally block resets its lease.
                pass
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

    tasks = [asyncio.create_task(listen()), asyncio.create_task(character()), asyncio.create_task(alarms())]
    try:
        async with websocket_serve(connection, args.host, args.port,
                                  max_size=4096, max_queue=16, compression=None, ping_interval=5, ping_timeout=5):
            await asyncio.gather(*tasks)
    finally:
        capture.close()
        if isinstance(direction, XvfDirectionEstimator):
            direction.close()
        for task in tasks: task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
        if wake_debug is not None:
            await asyncio.to_thread(wake_debug.close)
        if session.session_id:
            with suppress(OSError, ValueError, asyncio.TimeoutError):
                await daemon_command(f"voice {session.session_id} cancel", args.daemon_socket)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=7448)
    parser.add_argument("--token-file", type=Path, required=True)
    parser.add_argument("--mute-file", type=Path, default=Path.home() / ".config/orion/microphone.json")
    parser.add_argument("--hardware", choices=("v1", "v2"), default="v1")
    parser.add_argument("--capture-channels", type=int, choices=(2, 6), default=2)
    parser.add_argument("--processed-channel", type=int, default=0, help="Verified XVF3800 processed USB channel; never mix raw channels")
    parser.add_argument("--device", default=None)
    parser.add_argument("--wake-model", type=Path, default=Path(__file__).resolve().parents[1] / "models/wake/hey_orion_reference.rpw")
    parser.add_argument("--threshold", type=float, default=0.35)
    parser.add_argument("--verifier-dir", type=Path, default=Path(__file__).resolve().parents[1] / "models/verifier",
                        help="openWakeWord phrase verifier; pass --no-verifier to use the ASR prefix instead")
    parser.add_argument("--no-verifier", dest="verifier_dir", action="store_const", const=None)
    parser.add_argument("--vad-model", type=Path, default=os.environ.get("ORION_VAD_MODEL"))
    parser.add_argument("--local-processor", action="store_true", help="Reserve processing ownership for the onboard coordinator")
    parser.add_argument("--mic-spacing", type=float, default=0.0)
    parser.add_argument("--channel-sign", type=int, choices=[-1, 0, 1], default=0)
    parser.add_argument("--xvf-direction", type=int, choices=(0, 1), default=os.environ.get("ORION_XVF_DIRECTION", "0"))
    parser.add_argument("--xvf-azimuth-offset-deg", type=float, default=os.environ.get("ORION_XVF_AZIMUTH_OFFSET_DEG"))
    parser.add_argument("--xvf-azimuth-sign", type=int, choices=(-1, 1), default=os.environ.get("ORION_XVF_AZIMUTH_SIGN"))
    parser.add_argument("--xvf-front-half-width-deg", type=float, default=os.environ.get("ORION_XVF_FRONT_HALF_WIDTH_DEG", "30"))
    parser.add_argument("--xvf-min-energy", type=float, default=os.environ.get("ORION_XVF_MIN_ENERGY", "0"))
    parser.add_argument("--daemon-socket", default="/tmp/oriond.sock")
    args = parser.parse_args()
    if not 0 < args.threshold <= 1:
        parser.error("--threshold must be in (0, 1]")
    if args.device is None:
        args.device = "plughw:CARD=Array,DEV=0" if args.hardware == "v2" else DEFAULT_CAPTURE_DEVICE
    if not 0 <= args.processed_channel < args.capture_channels:
        parser.error("--processed-channel must be within --capture-channels")
    if args.hardware == "v1" and args.capture_channels != 2:
        parser.error("V1 requires two raw microphone channels")
    asyncio.run(serve(args))


if __name__ == "__main__":
    main()
