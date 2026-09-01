from __future__ import annotations

import argparse
import asyncio
from contextlib import suppress
from dataclasses import dataclass
import hmac
from pathlib import Path
import signal
import time

from . import PROTOCOL_VERSION
from .agent import AgentProvider, CodexAgentProvider
from .models import DEFAULT_ASR_MODEL, DEFAULT_TTS_MODEL
from .protocol import ProtocolError, event, parse_hello, parse_json_message
from .providers import Qwen3AsrTranscriber
from .session import PendingTranscription, SessionEvent, VoiceSession
from .tts import ChatterboxSynthesizer
from .wake import RustpotterWakeDetector


@dataclass(frozen=True)
class VoiceModels:
    asr: Qwen3AsrTranscriber
    wake: RustpotterWakeDetector
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
) -> None:
    started = time.monotonic()
    try:
        result = await asyncio.to_thread(session.asr.transcribe, pending.pcm)
        duration_ms = round((time.monotonic() - started) * 1_000)
        command: str | None = None
        for message in session.complete_transcription(pending.purpose, result):
            extra = {
                "language": result.language,
                "durationMs": duration_ms,
            } if message.type == "transcript.final" else {}
            await send_session_event(websocket, message, **extra)
            if message.type == "transcript.final":
                command = str(message.fields["text"])
        if command is not None:
            await generate_response(websocket, session, models, command, request_id)
    except Exception as error:
        with suppress(ValueError):
            session.fail_transcription(pending.purpose)
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
) -> None:
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
        await websocket.send(event(
            "worker.error",
            code="agent_failed",
            message=str(error),
            recoverable=True,
        ))
        return

    try:
        await websocket.send(event("synthesis.started", requestId=request_id))
        started = time.monotonic()
        audio = await asyncio.to_thread(models.tts.synthesize, response)
        session.begin_playback(request_id)
        await websocket.send(event(
            "speech.audio",
            requestId=request_id,
            sampleRate=audio.sample_rate,
            samples=audio.samples,
            durationMs=audio.duration_ms,
            synthesisMs=round((time.monotonic() - started) * 1_000),
        ))
        await websocket.send(audio.pcm)
    except Exception as error:
        with suppress(ValueError):
            session.fail_response()
        await websocket.send(event(
            "worker.error",
            code="synthesis_failed",
            message=str(error),
            recoverable=True,
        ))


async def handle_connection(websocket: object, token: str, models: VoiceModels) -> None:
    transcription: asyncio.Task[None] | None = None
    next_request_id = 1
    try:
        first = await asyncio.wait_for(websocket.recv(), timeout=10)
        if not isinstance(first, str):
            raise ProtocolError("The first message must be a JSON handshake.")
        hello = parse_hello(first)
        if not hmac.compare_digest(hello.token, token):
            await websocket.close(4003, "Invalid worker token")
            return

        session = VoiceSession(models.asr, models.wake)
        await websocket.send(event(
            "ready",
            protocol=PROTOCOL_VERSION,
            asr={"provider": models.asr.provider, "model": models.asr.model_name},
            wake={
                "provider": models.wake.provider,
                "model": models.wake.model_name,
                "threshold": models.wake.threshold,
            },
            agent={"provider": models.agent.provider, "model": models.agent.model_name},
            tts={"provider": models.tts.provider, "model": models.tts.model_name},
        ))

        async for raw in websocket:
            if isinstance(raw, bytes):
                events, pending = session.accept_audio(raw)
                for session_event in events:
                    await send_session_event(websocket, session_event)
                if pending is not None:
                    await websocket.send(event("transcription.started", purpose=pending.purpose.value))
                    transcription = asyncio.create_task(
                        transcribe_utterance(
                            websocket,
                            session,
                            pending,
                            models,
                            next_request_id,
                        )
                    )
                    next_request_id += 1
                continue

            message = parse_json_message(raw)
            if message["type"] == "stop":
                await websocket.close(1000, "Studio voice stopped")
                return
            if message["type"] in {"playback.finished", "playback.failed"}:
                request_id = message.get("requestId")
                if not isinstance(request_id, int) or isinstance(request_id, bool):
                    raise ProtocolError("Playback acknowledgement requires an integer requestId.")
                try:
                    session.finish_playback(request_id)
                except ValueError as error:
                    raise ProtocolError(str(error)) from error
                await websocket.send(event("speech.completed", requestId=request_id))
                if message["type"] == "playback.failed":
                    detail = message.get("message")
                    await websocket.send(event(
                        "worker.error",
                        code="playback_failed",
                        message=detail if isinstance(detail, str) else "Studio could not play speech audio.",
                        recoverable=True,
                    ))
                continue
            raise ProtocolError(f"Unsupported client message {message['type']!r}.")
    except ProtocolError as error:
        await websocket.send(event(
            "worker.error", code="protocol_error", message=str(error), recoverable=False,
        ))
        await websocket.close(4002, "Protocol error")
    finally:
        if transcription:
            transcription.cancel()
            with suppress(asyncio.CancelledError):
                await transcription


def load_models(args: argparse.Namespace) -> VoiceModels:
    if args.agent_provider != "codex":
        raise RuntimeError(f"Unsupported agent provider: {args.agent_provider}")
    return VoiceModels(
        asr=Qwen3AsrTranscriber(args.asr_model),
        wake=RustpotterWakeDetector(Path(args.wake_model), args.wake_threshold),
        agent=CodexAgentProvider(args.agent_model),
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
            await handle_connection(websocket, args.token, models)

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
    parser.add_argument("--wake-model", required=True)
    parser.add_argument("--wake-threshold", type=float, default=0.4)
    parser.add_argument("--agent-provider", default="codex")
    parser.add_argument("--agent-model")
    parser.add_argument("--tts-model", default=DEFAULT_TTS_MODEL)
    return parser


def main() -> None:
    args = argument_parser().parse_args()
    if args.host not in {"127.0.0.1", "localhost", "::1"}:
        raise SystemExit("The voice worker may bind only to a loopback address.")
    if not 0 < args.wake_threshold <= 1:
        raise SystemExit("The wake threshold must be in the range (0, 1].")
    asyncio.run(serve(args))


if __name__ == "__main__":
    main()
