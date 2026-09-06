import asyncio
import threading
import unittest

from orion_voice_worker.speech_pipeline import GeneratedChunk, StartupBuffer, buffered_chunks
from orion_voice_worker.tts import SpeechAudio


def audio(value=1):
    return SpeechAudio(bytes([value, 0]) * 24000, 24000)


class BufferPolicyTests(unittest.TestCase):
    def test_fast_generation_reserves_six_seconds(self):
        policy = StartupBuffer()
        for _ in range(5):
            self.assertFalse(policy.add(GeneratedChunk(audio(), 500, 0)))
        self.assertTrue(policy.add(GeneratedChunk(audio(), 500, 0)))

    def test_slow_generation_selects_complete_reply_even_if_it_recovers(self):
        policy = StartupBuffer()
        for _ in range(6):
            self.assertFalse(policy.add(GeneratedChunk(audio(), 1100, 0)))
        for _ in range(20):
            self.assertFalse(policy.add(GeneratedChunk(audio(), 100, 0)))

    def test_long_generation_gap_increases_startup_reserve(self):
        policy = StartupBuffer()
        self.assertFalse(policy.add(GeneratedChunk(audio(), 3000, 0)))
        for _ in range(6):
            self.assertFalse(policy.add(GeneratedChunk(audio(), 100, 0)))
        self.assertTrue(policy.add(GeneratedChunk(audio(), 100, 0)))

    def test_progressively_slower_arrivals_choose_complete_buffering(self):
        # Approximate the incident's growing gaps with 0.8-second audio chunks.
        policy = StartupBuffer()
        chunk_audio = SpeechAudio(bytes(38400), 24000)
        for delay in [.5, .52, .56, .62, .72, .76, 1.04, .88, 1.0, 1.1, 1.2, 1.58]:
            self.assertFalse(policy.add(GeneratedChunk(chunk_audio, delay * 1000, 0)))
        self.assertTrue(policy.complete_only)


class PipelineTests(unittest.IsolatedAsyncioTestCase):
    async def test_upload_pause_does_not_stop_generation_but_queue_is_bounded(self):
        generated = []
        closed = []
        def source():
            try:
                for i in range(1, 31):
                    generated.append(i)
                    yield audio(i)
            finally:
                closed.append(True)
        chunks = buffered_chunks(source(), 1)
        first = await anext(chunks)
        self.assertEqual(first.audio.pcm[0], 1)
        await asyncio.sleep(.1)  # Consumer stands in for a blocked HTTP upload.
        self.assertGreater(len(generated), 6)
        self.assertLessEqual(len(generated), 15)  # six held + eight queued + one pending
        received = [first.audio.pcm[0]]
        async for chunk in chunks:
            received.append(chunk.audio.pcm[0])
        self.assertEqual(received, list(range(1, 31)))
        self.assertEqual(closed, [True])

    async def test_slow_source_finishes_before_first_upload(self):
        now = [0.0]
        done = []
        def source():
            for _ in range(10):
                now[0] += 1.2
                yield audio()
            done.append(True)
        chunks = buffered_chunks(source(), 2, clock=lambda: now[0])
        try:
            await anext(chunks)
            self.assertEqual(done, [True])
            self.assertEqual(len([chunk async for chunk in chunks]), 9)
        finally:
            await chunks.aclose()

    async def test_generation_failure_before_start_does_not_upload_partial_reply(self):
        def source():
            yield audio()
            raise RuntimeError('decoder failed')
        chunks = buffered_chunks(source(), 3)
        with self.assertRaisesRegex(RuntimeError, 'decoder failed'):
            await anext(chunks)

    async def test_cancel_waits_for_native_inference_before_closing_generator(self):
        entered, release, closed = threading.Event(), threading.Event(), threading.Event()
        def source():
            try:
                entered.set()
                release.wait(2)
                yield audio()
            finally:
                closed.set()
        chunks = buffered_chunks(source(), 4)
        pending = asyncio.create_task(anext(chunks))
        try:
            self.assertTrue(await asyncio.to_thread(entered.wait, 1))
            pending.cancel()
            await asyncio.sleep(.02)
            self.assertFalse(closed.is_set())
            release.set()
            with self.assertRaises(asyncio.CancelledError):
                await pending
            self.assertTrue(closed.is_set())
        finally:
            release.set()
            await asyncio.gather(pending, return_exceptions=True)
            await chunks.aclose()

    async def test_complete_buffering_retains_the_120_second_limit(self):
        now = [0.0]
        def source():
            for _ in range(121):
                now[0] += 2
                yield audio()
        chunks = buffered_chunks(source(), 5, clock=lambda: now[0])
        with self.assertRaisesRegex(RuntimeError, '120-second'):
            await anext(chunks)
