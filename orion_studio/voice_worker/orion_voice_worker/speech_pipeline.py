"""Bounded inference/upload overlap and conservative speech startup buffering."""
from __future__ import annotations

import asyncio
from contextlib import suppress
from dataclasses import dataclass
import json
import sys
import time

from .tts import SpeechAudio


def diagnostic(kind, **fields):
    # Never include reply text, PCM, or credentials in timing logs.
    print(json.dumps({"event": kind, **fields}), file=sys.stderr, flush=True)


@dataclass(frozen=True)
class GeneratedChunk:
    audio: SpeechAudio
    generation_ms: float
    synthesis_ms: float


@dataclass
class StartupBuffer:
    audio_seconds: float = 0
    generation_seconds: float = 0
    longest_step: float = 0
    complete_only: bool = False

    def add(self, chunk: GeneratedChunk) -> bool:
        self.audio_seconds += chunk.audio.samples / chunk.audio.sample_rate
        self.generation_seconds += chunk.generation_ms / 1000
        self.longest_step = max(self.longest_step, chunk.generation_ms / 1000)
        target = max(6.0, 2 * self.longest_step + 2)
        # Reserve 25% throughput headroom. A slow initial sample or a long
        # decoder pause selects complete-reply buffering for this entire run.
        if self.audio_seconds >= 6 and (
            self.generation_seconds / self.audio_seconds > .75 or target > 12
        ):
            self.complete_only = True
        return not self.complete_only and self.audio_seconds >= target


async def buffered_chunks(stream, request_id, clock=time.monotonic):
    """Yield ordered chunks while one producer alone owns the model iterator.

    Startup holds at most the 120-second reply limit (~5.8 MB PCM). After
    startup, eight transport chunks bound generation's lead over uploading.
    """
    queue = asyncio.Queue(maxsize=8)
    started = clock()

    async def produce():
        total_samples = 0
        try:
            while True:
                before = clock()
                pending = asyncio.create_task(asyncio.to_thread(next, stream, None))
                try:
                    audio = await asyncio.shield(pending)
                except asyncio.CancelledError:
                    # Python cancellation cannot stop native inference. Wait
                    # before closing/reusing its generator or model.
                    with suppress(Exception):
                        await pending
                    raise
                generated = clock()
                if audio is None:
                    await queue.put(None)
                    return
                if audio.sample_rate != 24000:
                    raise RuntimeError("Streaming playback requires Chatterbox PCM16 at 24 kHz")
                if not audio.pcm or len(audio.pcm) % 2 or audio.samples > 48000:
                    raise RuntimeError("Invalid or oversized synthesis chunk")
                total_samples += audio.samples
                if total_samples > 120 * 24000:
                    raise RuntimeError("Synthesized reply exceeds the 120-second playback limit")
                await queue.put(GeneratedChunk(audio, (generated - before) * 1000,
                                               (generated - started) * 1000))
        except Exception as error:
            await queue.put(error)
        finally:
            if hasattr(stream, "close"):
                await asyncio.to_thread(stream.close)

    producer = asyncio.create_task(produce())
    startup = StartupBuffer()
    held = []
    released = False
    try:
        while True:
            item = await queue.get()
            if isinstance(item, Exception):
                raise item
            if not released:
                ready = False
                if item is not None:
                    held.append(item)
                    ready = startup.add(item)
                if ready or item is None:
                    diagnostic("speech.buffer_ready", request_id=request_id,
                               mode="complete" if item is None else "stream",
                               audio_ms=round(startup.audio_seconds * 1000),
                               generation_ms=round(startup.generation_seconds * 1000))
                    released = True
                    for chunk in held:
                        yield chunk
                    held.clear()
            elif item is not None:
                yield item
            if item is None:
                return
    finally:
        producer.cancel()
        await asyncio.gather(producer, return_exceptions=True)
