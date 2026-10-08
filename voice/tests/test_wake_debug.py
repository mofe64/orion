from contextlib import redirect_stdout
from datetime import datetime, timedelta, timezone
import io
import json
from pathlib import Path
import tempfile
import threading
import time
import unittest
from unittest.mock import patch
import wave

import numpy as np

from orion_voice.satellite import SatelliteSession
from orion_voice.verifier import Verdict
from orion_voice.wake_debug import CaptureReadStats, WakeDebugRecorder
from test_satellite import Wake, frame
from test_verifier import verifier


class WakeDebugTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.directory = Path(directory.name) / 'debug'
        self.now = 100.0

    def session(self, scores=(), enabled=True):
        with patch.dict('os.environ', {'ORION_WAKE_DEBUG_DIR': str(self.directory) if enabled else ''}):
            recorder = WakeDebugRecorder.from_environment()
        if recorder is not None:
            self.addCleanup(recorder.close)
            recorder.clock = lambda: self.now
        self.wake = Wake()
        self.verifier = verifier(scores)
        session = SatelliteSession(self.wake, verifier=self.verifier, wake_debug=recorder,
                                   clock=lambda: self.now, barge_in=True)
        session.restart_verifier()
        return session

    def files(self, session):
        session.wake_debug._queue.join()
        folders = sorted(self.directory.iterdir())
        self.assertEqual(len(folders), 1)
        folder = folders[0]
        with wave.open(str(folder / 'audio.wav'), 'rb') as wav:
            self.assertEqual((wav.getnchannels(), wav.getsampwidth(), wav.getframerate()), (1, 2, 16000))
            pcm = wav.readframes(wav.getnframes())
        return folder, pcm, json.loads((folder / 'meta.json').read_text())

    def test_environment_unset_never_creates_debug_files_or_writer(self):
        with patch.dict('os.environ', {}, clear=True):
            self.assertIsNone(WakeDebugRecorder.from_environment())
        session = self.session({1: .9}, enabled=False)
        for _ in range(4): session.accept_stereo(frame())
        self.wake.next = True
        self.assertTrue(session.accept_stereo(frame())[-1]['accepted'])
        self.assertIsNone(session.wake_debug)
        self.assertFalse(self.directory.exists())

    def test_accepted_snapshot_is_exact_last_four_seconds_with_aligned_metadata(self):
        session = self.session({54: .9})
        expected = bytearray()
        for index in range(220):
            # Different channels and negative odd sums exercise the actual downmix.
            left = np.tile([index + 1000, -index - 1001], 160)
            right = left // 2
            stereo = np.column_stack((left, right)).astype('<i2')
            expected.extend((stereo.astype(np.int32).sum(axis=1) // 2).astype('<i2').tobytes())
            self.now += .02
            if index == 219: self.wake.next = True
            events = session.accept_stereo(stereo.tobytes())
        self.assertTrue(events[-1]['accepted'])
        folder, pcm, meta = self.files(session)
        self.assertTrue(folder.name.endswith('-accepted'))
        self.assertEqual(pcm, bytes(expected[-4 * 32000:]))
        self.assertEqual(meta['session_id'], session.session_id)
        self.assertEqual(meta['phase_at_candidate'], 'listening')
        self.assertEqual(meta['rustpotter_score'], .7)
        self.assertEqual(meta['candidate_position'], 220 * 320)
        self.assertEqual(meta['audio_start_sample'], 20 * 320)
        self.assertEqual(meta['audio_end_sample'], 220 * 320)
        self.assertEqual(meta['verdict']['decided_sample'], meta['candidate_position'])
        self.assertTrue(meta['verdict']['accepted'])
        self.assertTrue(meta['verifier_healthy'])
        self.assertAlmostEqual(meta['capture_open_seconds'], 4.4)
        self.assertEqual(len(meta['verifier_chunks']), 50)
        self.assertEqual(meta['verifier_chunk_seconds'], list(self.verifier.chunk_seconds))
        self.assertEqual(meta['verifier_chunks'][-2], {'end_sample': 54 * 1280, 'score': .9, 'eligible': True})

    def test_rejected_snapshot_includes_ineligible_scores_and_verification_tail(self):
        session = self.session()
        self.verifier.scorer.warmup = 26
        for _ in range(4): session.accept_stereo(frame(1234))
        self.wake.next = True
        self.assertEqual(session.accept_stereo(frame(2345))[0]['type'], 'wake.candidate')
        for _ in range(50):
            events = session.accept_stereo(frame(3456))
        self.assertFalse(events[0]['accepted'])
        folder, pcm, meta = self.files(session)
        self.assertTrue(folder.name.endswith('-rejected'))
        self.assertFalse(meta['verdict']['accepted'])
        self.assertEqual(meta['candidate_position'], 5 * 320)
        self.assertEqual(meta['verdict']['decided_sample'], 5 * 320 + 16000)
        self.assertEqual(meta['audio_end_sample'], 55 * 320)
        self.assertTrue(all(not chunk['eligible'] for chunk in meta['verifier_chunks']))
        samples = np.frombuffer(pcm, dtype='<i2')
        for value in (1234, 2345, 3456): self.assertIn(value, samples)

    def test_retention_caps_recordings_at_fifty_and_preserves_unrelated_folders(self):
        self.directory.mkdir()
        base = datetime(2020, 1, 1, tzinfo=timezone.utc)
        old = []
        for index in range(50):
            path = self.directory / ((base + timedelta(seconds=index)).strftime('%Y%m%dT%H%M%S%fZ') + '-rejected')
            path.mkdir()
            (path / 'audio.wav').write_bytes(b'old')
            old.append(path)
        unrelated = self.directory / 'keep-me'
        unrelated.mkdir()
        session = self.session({1: .9})
        for _ in range(4): session.accept_stereo(frame())
        self.wake.next = True
        session.accept_stereo(frame())
        session.wake_debug._queue.join()
        self.assertEqual(len(list(self.directory.iterdir())), 51)
        self.assertFalse(old[0].exists())
        self.assertTrue(all(path.exists() for path in old[1:]))
        self.assertTrue(unrelated.exists())

    def test_alarm_dismissal_and_cancelled_candidate_never_write(self):
        session = self.session({3: .9})
        self.wake.next = True
        session.accept_stereo(frame())
        session.set_alarm(True)  # Cancel a still-pending verifier candidate.
        self.wake.next = True
        self.assertEqual(session.accept_stereo(frame()), [{'type': 'alarm.dismiss'}])
        for _ in range(20): self.assertEqual(session.accept_stereo(frame()), [])
        session.wake_debug.close()
        self.assertFalse(self.directory.exists())

    def test_barge_in_rejection_also_records_candidate_in_playing_phase(self):
        session = self.session()
        session.session_id = 'a' * 32
        session.phase = 'playing'
        self.wake.next = True
        for _ in range(51): self.assertEqual(session.accept_stereo(frame()), [])
        folder, _, meta = self.files(session)
        self.assertTrue(folder.name.endswith('-rejected'))
        self.assertEqual(meta['phase_at_candidate'], 'playing')
        self.assertEqual(meta['session_id'], 'a' * 32)
        self.assertEqual(session.phase, 'playing')

    def test_accepted_barge_in_saves_snapshot_before_resetting_the_old_session(self):
        session = self.session({1: .9})
        session.session_id = 'a' * 32
        session.phase = 'playing'
        for _ in range(4): session.accept_stereo(frame(1234))
        self.wake.next = True
        events = session.accept_stereo(frame(2345))
        self.assertEqual(events[0]['type'], 'session.interrupted')
        self.assertNotEqual(session.session_id, 'a' * 32)
        folder, pcm, meta = self.files(session)
        self.assertTrue(folder.name.endswith('-accepted'))
        self.assertEqual(meta['session_id'], 'a' * 32)
        self.assertEqual(meta['phase_at_candidate'], 'playing')
        self.assertEqual(meta['candidate_position'], 5 * 320)
        self.assertEqual(np.frombuffer(pcm, dtype='<i2')[0], 1234)

    def test_slow_writer_and_full_queue_do_not_block_wake_decisions(self):
        session = self.session({1: .9})
        recorder = session.wake_debug
        entered, release = threading.Event(), threading.Event()
        write = recorder._write
        def slow_write(*args):
            entered.set()
            release.wait(2)
            write(*args)
        recorder._write = slow_write
        try:
            for _ in range(4): session.accept_stereo(frame())
            self.wake.next = True
            started = time.monotonic()
            events = session.accept_stereo(frame())
            self.assertLess(time.monotonic() - started, .5)
            self.assertTrue(events[-1]['accepted'])
            self.assertTrue(entered.wait(1))
            # Fill the writer queue while disk is blocked, then drop one snapshot.
            with self.assertLogs(level='WARNING') as logs:
                for _ in range(9):
                    recorder.begin(session.session_id, 'listening', .7, self.verifier.position)
                    recorder.verdict(Verdict(True, .9, self.verifier.position), self.verifier)
            self.assertEqual(recorder._queue.qsize(), 8)
            self.assertIn('queue full', logs.output[0])
            self.assertEqual(session.phase, 'wake')
        finally:
            release.set()
            recorder.close()

    def test_disk_error_does_not_change_verdict_or_session(self):
        self.directory.write_text('not a directory')
        session = self.session({1: .9})
        for _ in range(4): session.accept_stereo(frame())
        self.wake.next = True
        with self.assertLogs(level='WARNING') as logs:
            events = session.accept_stereo(frame())
            session.wake_debug._queue.join()
        self.assertTrue(events[-1]['accepted'])
        self.assertEqual(session.phase, 'wake')
        self.assertIn('recording failed', logs.output[0])

    def test_capture_reopen_discards_previous_audio_and_resets_sample_origin(self):
        session = self.session()
        for _ in range(10): session.accept_stereo(frame(1234))
        self.now += 20
        session.restart_verifier()
        self.verifier.scorer.scores = {1: .9}
        for _ in range(4): session.accept_stereo(frame(2345))
        self.wake.next = True
        session.accept_stereo(frame(3456))
        _, pcm, meta = self.files(session)
        self.assertEqual(meta['audio_start_sample'], 0)
        self.assertEqual(meta['audio_end_sample'], 5 * 320)
        self.assertEqual(meta['capture_open_seconds'], 0)
        self.assertNotIn(1234, np.frombuffer(pcm, dtype='<i2'))

    def test_last_250_processing_times_are_snapshotted(self):
        session = self.session({260: .9})
        for _ in range(260 * 4): session.accept_stereo(frame())
        self.wake.next = True
        session.accept_stereo(frame())
        _, _, meta = self.files(session)
        self.assertEqual(len(meta['verifier_chunk_seconds']), 250)
        self.assertEqual(len(meta['verifier_chunks']), 50)


class CaptureReadStatsTests(unittest.TestCase):
    def test_once_per_minute_counts_only_reads_over_forty_ms_and_resets(self):
        now = 0
        stats = CaptureReadStats(clock=lambda: now)
        output = io.StringIO()
        with redirect_stdout(output):
            stats.accept(.04)
            stats.accept(.041)
            now = 59.9
            stats.accept(.02)
            self.assertEqual(output.getvalue(), '')
            now = 60
            stats.accept(.08)
            now = 120
            stats.accept(.02)
        logs = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual(logs[0], {'event': 'voice.capture_read_timing', 'reads': 4,
            'reads_over_40_ms': 2, 'window_seconds': 60})
        self.assertEqual(logs[1]['reads'], 1)
        self.assertEqual(logs[1]['reads_over_40_ms'], 0)
