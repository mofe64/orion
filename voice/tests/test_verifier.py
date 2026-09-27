import importlib.util
import json
from pathlib import Path
import unittest

import numpy as np

from orion_voice.satellite import SatelliteSession
from orion_voice.verifier import CHUNK, RATE, AcousticVerifier, sha256

from test_satellite import Wake, frame

MODELS = Path(__file__).resolve().parents[1] / "models/verifier"


class ScriptedScorer:
    """Replay chosen scores at chunk ends, like StreamingScorer without ONNX."""
    def __init__(self, scores=(), warmup=0):
        self.scores = dict(scores)  # chunk number -> score
        self.warmup = warmup
        self.reset()

    def reset(self):
        self.frames = self.samples = 0
        self._pending = np.empty(0, dtype=np.int16)

    def feed(self, pcm):
        self._pending = np.concatenate((self._pending, pcm))
        out = []
        while len(self._pending) >= CHUNK:
            self._pending = self._pending[CHUNK:]
            self.frames += 1
            self.samples += CHUNK
            out.append((self.samples, self.scores.get(self.frames, 0.0), self.frames > self.warmup))
        return out


def verifier(scores=(), warmup=0):
    return AcousticVerifier(MODELS, scorer=ScriptedScorer(scores, warmup))


def feed_frames(v, count):
    for _ in range(count):
        v.feed(np.zeros(320, dtype=np.int16))


class VerifierRuleTests(unittest.TestCase):
    def test_config_matches_gate2_selection(self):
        config = json.loads((MODELS / "config.json").read_text())
        self.assertEqual((config["threshold"], config["lookback_seconds"], config["deadline_seconds"]), (0.2, 0.8, 1.0))
        for name, digest in config["files"].items():
            self.assertEqual(sha256(MODELS / name), digest)

    def test_score_before_candidate_accepts_at_candidate(self):
        v = verifier({30: 0.9})
        feed_frames(v, 40 * 4)  # 40 chunks; chunk 30 ends 0.8 s before chunk 40
        v.begin()
        verdict = v.verdict()
        self.assertTrue(verdict.accepted)
        self.assertEqual(verdict.decided_sample, v.candidate or 40 * CHUNK)

    def test_score_older_than_lookback_does_not_count(self):
        v = verifier({29: 0.9})
        feed_frames(v, 40 * 4)
        v.begin()
        self.assertIsNone(v.verdict())

    def test_later_score_within_deadline_accepts_when_it_arrives(self):
        v = verifier({45: 0.5})
        feed_frames(v, 40 * 4)
        v.begin()
        for _ in range(4 * 5 - 1):
            feed_frames(v, 1)
            self.assertIsNone(v.verdict())
        feed_frames(v, 1)
        verdict = v.verdict()
        self.assertTrue(verdict.accepted)
        self.assertEqual(verdict.decided_sample, 45 * CHUNK)

    def test_rejects_at_deadline_and_reports_best_score(self):
        v = verifier({41: 0.19, 60: 0.9})  # 60 is after the 1 s deadline
        feed_frames(v, 40 * 4)
        v.begin()
        verdict = None
        frames = 0
        while verdict is None:
            feed_frames(v, 1)
            frames += 1
            verdict = v.verdict()
        self.assertFalse(verdict.accepted)
        self.assertAlmostEqual(verdict.score, 0.19)
        self.assertEqual(frames * 320, RATE)

    def test_warmup_scores_are_ignored(self):
        v = verifier({3: 0.99}, warmup=26)
        feed_frames(v, 4 * 4)
        v.begin()
        feed_frames(v, 50)
        self.assertFalse(v.verdict().accepted)

    def test_slow_chunks_mark_verifier_unhealthy(self):
        ticks = iter(np.arange(0, 1000, 0.05))
        v = AcousticVerifier(MODELS, scorer=ScriptedScorer(), clock=lambda: next(ticks))
        feed_frames(v, 250 * 4)
        self.assertFalse(v.healthy)


class SessionVerifierTests(unittest.TestCase):
    def setUp(self):
        self.wake = Wake()

    def session(self, scores):
        self.verifier = verifier(scores)
        return SatelliteSession(self.wake, clock=lambda: 0, verifier=self.verifier)

    def trigger(self, session, before=40 * 4):
        for _ in range(before - 1): session.accept_stereo(frame(1234))
        self.wake.next = True
        return session.accept_stereo(frame())

    def test_accepted_candidate_skips_prefix_and_reports_verdict_immediately(self):
        session = self.session({39: 0.8})
        session.early_wake = True
        result = self.trigger(session)
        self.assertEqual([m['type'] for m in result], ['wake.candidate', 'wake.verified'])
        self.assertTrue(result[0]['acousticVerification'])
        self.assertTrue(result[1]['accepted'])
        self.assertEqual(result[1]['source'], 'acoustic')
        self.assertEqual(result[1]['verifierMs'], 0)
        for _ in range(30):
            for message in session.accept_stereo(frame(2100)):
                self.assertNotEqual(message.get('purpose'), 'wake_prefix')

    def test_rejected_candidate_holds_endpoint_until_verdict(self):
        session = self.session({})
        result = self.trigger(session)
        self.assertEqual(len(result), 1)
        seen = []
        for _ in range(49):
            seen += session.accept_stereo(frame(3000))
        seen += session.accept_stereo(frame(0))
        kinds = [m['type'] for m in seen if isinstance(m, dict)]
        self.assertEqual(kinds[0], 'wake.verified')
        self.assertFalse(seen[0]['accepted'])
        self.assertIs(session.acoustic_verdict, False)

    def test_unhealthy_verifier_falls_back_to_asr_prefix(self):
        session = self.session({39: 0.8})
        self.verifier.healthy = False
        session.early_wake = True
        result = self.trigger(session)
        self.assertFalse(result[0]['acousticVerification'])
        self.assertTrue(session.prefix_pending)

    def test_reset_cancels_pending_verification(self):
        session = self.session({})
        self.trigger(session)
        session.reset()
        self.assertFalse(session.acoustic_pending)
        self.assertIsNone(self.verifier.candidate)


@unittest.skipUnless(importlib.util.find_spec("onnxruntime"), "onnxruntime is installed only in the Pi voice environment")
class PackagedModelTests(unittest.TestCase):
    def test_chunking_does_not_change_scores(self):
        rng = np.random.default_rng(7)
        audio = (rng.standard_normal(RATE * 4) * 800).astype(np.int16)
        streamed = AcousticVerifier(MODELS)
        buffered = AcousticVerifier(MODELS)
        for start in range(0, len(audio), 320):
            streamed.feed(audio[start:start + 320])
        buffered.feed(audio)
        self.assertEqual(len(streamed.history), len(buffered.history))
        for (a, x), (b, y) in zip(streamed.history, buffered.history):
            self.assertEqual(a, b)
            self.assertLess(abs(x - y), 1e-6)
        self.assertTrue(all(0 <= score <= 1 for _, score in streamed.history))


if __name__ == "__main__":
    unittest.main()
