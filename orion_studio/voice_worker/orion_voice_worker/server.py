from __future__ import annotations

import argparse
import asyncio
from contextlib import suppress
from dataclasses import dataclass
import hmac
import signal
import time
import os
from urllib.parse import urlparse
from concurrent.futures import ThreadPoolExecutor

from . import PROTOCOL_VERSION
from .agent import AgentProvider, CodexAgentProvider, DEFAULT_AGENT_MODEL, DEFAULT_AGENT_EFFORT
from .models import DEFAULT_ASR_MODEL, DEFAULT_TTS_MODEL
from .protocol import ProtocolError, event, parse_hello, parse_json_message
from .providers import Qwen3AsrTranscriber
from .session import PendingTranscription, SessionEvent, VoiceSession
from .tts import ChatterboxSynthesizer


@dataclass(frozen=True)
class VoiceModels:
    asr: Qwen3AsrTranscriber
    agent: AgentProvider
    tts: ChatterboxSynthesizer


async def send_session_event(websocket: object, message: SessionEvent, **extra: object) -> None:
    await websocket.send(event(message.type, **message.fields, **extra))


async def transcribe_utterance(
    websocket: object,
    session: VoiceSession,
    pending: PendingTranscription,
    models: VoiceModels,
    request_id: int,
    pi=None,
) -> None:
    started = time.monotonic()
    session_id = session.session_id
    try:
        result = await asyncio.to_thread(session.asr.transcribe, pending.pcm)
        duration_ms = round((time.monotonic() - started) * 1_000)
        await websocket.send(event("stage.timing", stage="transcription_" + pending.purpose.value, durationMs=duration_ms))
        command: str | None = None
        for message in session.complete_transcription(pending.purpose, result):
            extra = {
                "language": result.language,
                "durationMs": duration_ms,
            } if message.type == "transcript.final" else {}
            if pi is not None and message.type in {"wake.confirmed", "wake.rejected"}:
                await pi.send(event("wake.confirmed" if message.type == "wake.confirmed" else "session.reject",
                                    sessionId=session_id, followup=not message.fields.get("hasCommand", False)))
            await send_session_event(websocket, message, **extra)
            if message.type == "transcript.final":
                command = str(message.fields["text"])
        if command is not None:
            if pi is not None:
                await pi.send(event("session.processing", sessionId=session_id))
            await generate_response(websocket, session, models, command, request_id, pi)
    except Exception as error:
        session.reset()
        if pi is not None:
            await pi.send(event("session.cancel", sessionId=session_id))
        await websocket.send(event(
            "worker.error",
            code="transcription_failed",
            message=str(error),
            recoverable=True,
        ))


async def generate_response(
    websocket: object,
    session: VoiceSession,
    models: VoiceModels,
    command: str,
    request_id: int,
    pi=None,
) -> None:
    session_id = session.session_id
    try:
        await websocket.send(event("agent.started", requestId=request_id))
        started = time.monotonic()
        response = await asyncio.to_thread(models.agent.respond, command)
        await websocket.send(event(
            "agent.response",
            requestId=request_id,
            text=response,
            durationMs=round((time.monotonic() - started) * 1_000),
        ))
    except Exception as error:
        session.fail_response()
        if pi is not None:
            await pi.send(event("session.cancel", sessionId=session_id))
        await websocket.send(event(
            "worker.error",
            code="agent_failed",
            message=str(error),
            recoverable=True,
        ))
        return

    chunks = None
    try:
        await websocket.send(event("synthesis.started", requestId=request_id))
        started = time.monotonic()
        chunks = iter(models.tts.stream(response))
        sequence = 0
        total_samples = 0
        while True:
            audio = await asyncio.to_thread(next, chunks, None)
            if audio is None:
                break
            if audio.sample_rate != 24000:
                raise RuntimeError("Streaming playback requires Chatterbox PCM16 at 24 kHz")
            total_samples += audio.samples
            if total_samples > 120 * 24000:
                raise RuntimeError("Synthesized reply exceeds the 120-second playback limit")
            if sequence == 0:
                session.begin_playback(request_id)
                if pi is not None:
                    await pi.send(event("session.playing", sessionId=session_id))
            await websocket.send(event("speech.chunk", requestId=request_id, sequence=sequence,
                sampleRate=audio.sample_rate, samples=audio.samples, durationMs=audio.duration_ms,
                synthesisMs=round((time.monotonic() - started) * 1000)))
            await websocket.send(audio.pcm)
            sequence += 1
        if sequence == 0:
            raise RuntimeError("Speech synthesis returned no chunks")
        session.synthesis_finished = True
        await websocket.send(event("speech.end", requestId=request_id, sequence=sequence,
            synthesisMs=round((time.monotonic() - started) * 1000)))
    except Exception as error:
        session.fail_response()
        if pi is not None:
            await pi.send(event("session.cancel", sessionId=session_id))
        await websocket.send(event(
            "worker.error",
            code="synthesis_failed",
            message=str(error),
            recoverable=True,
        ))

    finally:
        if chunks is not None and hasattr(chunks, "close"):
            await asyncio.to_thread(chunks.close)


def validate_pi_url(url: str) -> None:
    parsed = urlparse(url)
    if parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise ValueError("Listener URL must not contain credentials, query or fragment")
    if parsed.scheme != "ws" or not parsed.hostname:
        raise ValueError("Pi listener URL must use ws:// with a hostname")


async def handle_connection(websocket, token, models, pi_url, pi_token):
    from websockets.asyncio.client import connect
    first = await asyncio.wait_for(websocket.recv(), 10)
    if not isinstance(first, str) or not hmac.compare_digest(parse_hello(first).token, token):
        await websocket.close(4003, "Invalid worker token")
        return
    validate_pi_url(pi_url)
    async with connect(pi_url, max_size=18 * 32000, max_queue=16,
                       compression=None, open_timeout=10) as pi:
        await pi.send(event("hello", protocol=1, token=pi_token))
        ready = parse_json_message(await asyncio.wait_for(pi.recv(), 10))
        if (ready.get("type") != "ready" or ready.get("protocol") != 1
                or ready.get("sampleRate") != 16000 or ready.get("channels") != 1
                or ready.get("encoding") != "pcm_s16le" or not isinstance(ready.get("wake"), dict)):
            raise ProtocolError("Unsupported Pi listener contract")
        session = VoiceSession(models.asr)
        await websocket.send(event("ready", protocol=PROTOCOL_VERSION,
            asr={"provider": models.asr.provider, "model": models.asr.model_name},
            wake=ready["wake"],
            agent={"provider": models.agent.provider, "model": models.agent.model_name,
                   "effort": getattr(models.agent, "effort", "medium"),
                   "runtime": getattr(models.agent, "runtime_path", "unknown"),
                   "models": getattr(models.agent, "available_models", [])},
            tts={"provider": models.tts.provider, "model": models.tts.model_name}))
        transcription = None
        next_request_id = 1

        async def receive_pi():
            nonlocal transcription, next_request_id
            async for raw in pi:
                if not isinstance(raw, str):
                    raise ProtocolError("Unexpected Pi audio frame")
                message = parse_json_message(raw)
                kind = message["type"]
                session_id = message.get("sessionId")
                if kind == "wake.candidate":
                    session.begin(session_id)
                    await websocket.send(raw)
                elif kind == "utterance":
                    size = message.get("bytes")
                    if type(size) is not int or not 0 < size <= 18 * 32000 or size % 2:
                        raise ProtocolError("Invalid Pi utterance length")
                    pcm = await asyncio.wait_for(pi.recv(), 5)
                    if not isinstance(pcm, bytes) or len(pcm) != size:
                        raise ProtocolError("Pi utterance does not match its metadata")
                    pending = session.accept_utterance(session_id, message.get("purpose"), pcm)
                    await websocket.send(event("transcription.started", purpose=pending.purpose.value, captureMs=message.get("captureMs")))
                    transcription = asyncio.create_task(transcribe_utterance(
                        websocket, session, pending, models, next_request_id, pi))
                    next_request_id += 1
                elif kind == "session.expired":
                    if session_id != session.session_id:
                        raise ProtocolError("Stale Pi expiry")
                    if transcription:
                        transcription.cancel()
                        await asyncio.gather(transcription, return_exceptions=True)
                    session.reset()
                    await websocket.send(event("worker.error", code="session_expired",
                        message="Orion voice session timed out. Re-enable the microphone to continue.", recoverable=False))
                    return
                else:
                    raise ProtocolError("Unknown Pi event")

        async def receive_studio():
            async for raw in websocket:
                if not isinstance(raw, str):
                    raise ProtocolError("Studio microphone capture is not supported; audio comes from Orion")
                message = parse_json_message(raw)
                if message["type"] == "stop":
                    return
                if message["type"] not in {"playback.finished", "playback.failed"} or type(message.get("requestId")) is not int:
                    raise ProtocolError("Invalid playback acknowledgement")
                session_id = session.session_id
                if message["type"] == "playback.finished" and not session.synthesis_finished:
                    raise ProtocolError("Playback cannot finish before synthesis ends")
                if message["type"] == "playback.failed" and transcription and not transcription.done():
                    transcription.cancel()
                    await asyncio.gather(transcription, return_exceptions=True)
                session.finish_playback(message["requestId"])
                await pi.send(event("session.finish", sessionId=session_id))
                await websocket.send(event("speech.completed", requestId=message["requestId"]))
                if message["type"] == "playback.failed":
                    await websocket.send(event("worker.error", code="playback_failed",
                        message=str(message.get("message", "Pi playback failed")), recoverable=True))

        tasks = [asyncio.create_task(receive_pi()), asyncio.create_task(receive_studio())]
        try:
            done, _ = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
            for task in done:
                task.result()
        finally:
            for task in tasks + ([transcription] if transcription else []):
                task.cancel()
            await asyncio.gather(*tasks, *([transcription] if transcription else []), return_exceptions=True)
            await websocket.close()


def load_models(args: argparse.Namespace) -> VoiceModels:
    if args.agent_provider != "codex":
        raise RuntimeError(f"Unsupported agent provider: {args.agent_provider}")
    return VoiceModels(
        asr=Qwen3AsrTranscriber(args.asr_model),
        agent=CodexAgentProvider(args.agent_model, effort=args.agent_effort),
        tts=ChatterboxSynthesizer(args.tts_model),
    )


async def serve(args: argparse.Namespace) -> None:
    try:
        from websockets.asyncio.server import serve as websocket_serve
    except ImportError as error:
        raise RuntimeError("Install the voice worker dependencies before starting it.") from error

    stop = asyncio.get_running_loop().create_future()
    for signal_name in (signal.SIGINT, signal.SIGTERM):
        with suppress(NotImplementedError):
            asyncio.get_running_loop().add_signal_handler(signal_name, stop.set_result, None)

    model_task = asyncio.create_task(asyncio.to_thread(load_models, args))
    connection_lock = asyncio.Lock()

    async def exclusive_connection(websocket: object) -> None:
        if connection_lock.locked():
            await websocket.send(event(
                "worker.error",
                code="worker_busy",
                message="Orion Studio already owns this voice worker.",
                recoverable=False,
            ))
            await websocket.close(4009, "Worker already in use")
            return
        async with connection_lock:
            try:
                models = await model_task
            except Exception as error:
                await websocket.send(event(
                    "worker.error",
                    code="model_load_failed",
                    message=str(error),
                    recoverable=False,
                ))
                await websocket.close(4010, "Model load failed")
                return
            try:
                await handle_connection(websocket, args.token, models, args.pi_url,
                                        os.environ["ORION_PI_VOICE_TOKEN"])
            except Exception as error:
                with suppress(Exception):
                    await websocket.send(event("worker.error", code="pi_connection_failed",
                        message=str(error), recoverable=False))
                    await websocket.close(4011, "Pi voice unavailable")

    try:
        async with websocket_serve(
            exclusive_connection,
            args.host,
            args.port,
            max_size=2 * 1024 * 1024,
            compression=None,
        ):
            print(f"Orion voice worker listening on ws://{args.host}:{args.port}", flush=True)
            await stop
    finally:
        if model_task.done() and not model_task.cancelled() and model_task.exception() is None:
            await asyncio.to_thread(model_task.result().agent.close)


def argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Run Orion Studio's local voice model worker.")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--token", required=True)
    parser.add_argument("--asr-model", default=DEFAULT_ASR_MODEL)
    parser.add_argument("--pi-url", required=True)
    parser.add_argument("--agent-provider", default="codex")
    parser.add_argument("--agent-model", default=DEFAULT_AGENT_MODEL)
    parser.add_argument("--agent-effort", default=DEFAULT_AGENT_EFFORT, choices=["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"])
    parser.add_argument("--tts-model", default=DEFAULT_TTS_MODEL)
    return parser


def main() -> None:
    args = argument_parser().parse_args()
    if args.host not in {"127.0.0.1", "localhost", "::1"}:
        raise SystemExit("The voice worker may bind only to a loopback address.")
    # A cancelled request may finish inside a model library. Serialize inference
    # so a following session cannot concurrently enter the same model instance.
    async def run():
        asyncio.get_running_loop().set_default_executor(ThreadPoolExecutor(max_workers=1))
        await serve(args)
    asyncio.run(run())


if __name__ == "__main__":
    main()
