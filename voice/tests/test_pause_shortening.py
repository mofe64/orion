from dataclasses import replace
import json
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import numpy as np

from orion_voice.endpoint import EnergyEndpointDetector
from orion_voice.satellite import FRAME_BYTES, WAKE_PREFIX_BYTES, SatelliteSession, shorten_pauses
from orion_voice.vad import SileroEndpoint
from test_satellite import Wake, frame
from test_vad import Model


def mono(value):
    return np.frombuffer(frame(value), dtype='<i2').reshape(-1, 2)[:, 0].tobytes()


class PauseShorteningTests(unittest.TestCase):
    def test_two_second_gap_keeps_every_speech_frame_in_order(self):
        # Distinct frames make any lost or reordered speech detectable.
        frames = [np.full(320, i, dtype='<i2').tobytes() for i in range(200)]
        decisions = [True] * 50 + [False] * 100 + [True] * 50
        audio = shorten_pauses(b''.join(frames), decisions)
        self.assertEqual(audio, b''.join(frames[:58] + frames[142:]))

    def test_eight_frames_before_speech_are_always_kept(self):
        for pause_frames in (8, 30, 31, 32, 100):
            with self.subTest(pause_frames=pause_frames):
                frames = [np.full(320, i, dtype='<i2').tobytes() for i in range(pause_frames + 10)]
                audio = shorten_pauses(b''.join(frames), [False] * pause_frames + [True] * 10)
                self.assertTrue(audio.endswith(b''.join(frames[pause_frames - 8:])))

    def test_leading_and_trailing_silence_keep_sixteen_frames_each(self):
        audio = mono(0) * 100 + mono(2000) * 50 + mono(0) * 60
        decisions = [False] * 100 + [True] * 50 + [False] * 60
        self.assertEqual(shorten_pauses(audio, decisions),
                         mono(0) * 16 + mono(2000) * 50 + mono(0) * 16)

    def test_clip_without_long_gaps_is_unchanged(self):
        audio = mono(0) * 30 + mono(2000) * 50 + mono(0) * 29
        self.assertEqual(shorten_pauses(audio, [False] * 30 + [True] * 50 + [False] * 29), audio)

    def test_unknown_pre_roll_is_unchanged_and_separates_silence_runs(self):
        audio = mono(0) * 150 + mono(2000) + mono(0) * 30
        decisions = [None] * 150 + [True] + [False] * 30
        self.assertEqual(shorten_pauses(audio, decisions), audio)

    def start_wake(self, **kwargs):
        wake = Wake()
        session = SatelliteSession(wake, **kwargs)
        for _ in range(149): session.accept_stereo(frame(0))
        wake.next = True
        session.accept_stereo(frame(1234))
        return session

    def test_wake_session_sends_full_three_seconds_of_pre_roll_with_aligned_decisions(self):
        session = SatelliteSession(Wake())
        for i in range(175):
            session.append_pre_roll(mono(1000 + i), (None, False, True)[i % 3])
        self.assertEqual(len(session.pre_roll), 3 * 32000)
        expected = bytes(session.pre_roll[-WAKE_PREFIX_BYTES:])
        decisions = session.pre_roll_speech[-(WAKE_PREFIX_BYTES // FRAME_BYTES):]
        session.start_wake(SimpleNamespace(name='hey_orion', score=.7))
        self.assertEqual(session.utterance, expected)
        self.assertEqual(session.utterance_speech, decisions)
        self.assertEqual(len(session.utterance_speech) * FRAME_BYTES, len(session.utterance))
        for _ in range(50): session.accept_stereo(frame(2000))
        for _ in range(50): messages = session.accept_stereo(frame(0))
        self.assertEqual(messages[0]['purpose'], 'wake_and_command')
        self.assertEqual(messages[1], expected + mono(2000) * 50 + mono(0) * 16)

    def test_listener_trims_gap_and_tail_but_preserves_pre_roll_and_capture_time(self):
        # A longer grace period lets this fixture capture the entire two-second gap.
        session = self.start_wake(endpoint_factory=lambda config:
            EnergyEndpointDetector(replace(config, min_capture_ms=5000)))
        speech = []
        for value, count in [(2000, 30), (0, 100), (3000, 50)]:
            for _ in range(count):
                self.assertEqual(session.accept_stereo(frame(value)), [])
                if value: speech.append(mono(value))
        with patch('builtins.print') as log:
            for _ in range(70):
                messages = session.accept_stereo(frame(0))
        metadata, audio = messages
        pre_roll = mono(0) * 149 + mono(1234)
        self.assertEqual(audio, pre_roll + b''.join(speech[:30]) + mono(0) * 16
                         + b''.join(speech[30:]) + mono(0) * 16)
        self.assertEqual(metadata['purpose'], 'wake_and_command')
        self.assertEqual(metadata['captureMs'], 5000)
        self.assertEqual(metadata['sentMs'], 5240)
        self.assertEqual(metadata['bytes'], len(audio))
        endpoint_log = json.loads(log.call_args.args[0])
        self.assertEqual(endpoint_log['capture_ms'], 5000)
        self.assertEqual(endpoint_log['sentMs'], 5240)

    def test_silero_decisions_preserve_quiet_speech_without_extra_inference(self):
        model = Model()
        session = self.start_wake(endpoint_factory=lambda config:
            SileroEndpoint(replace(config, trailing_silence_ms=1200), model))
        pre_roll = bytes(session.utterance)
        for _ in range(50): self.assertEqual(session.accept_stereo(frame(300)), [])
        model.probability = .01
        for _ in range(100):
            messages = session.accept_stereo(frame(0))
            if messages: break
        metadata, audio = messages
        self.assertTrue(audio.startswith(pre_roll + mono(300) * 50))
        self.assertEqual(audio[-16 * FRAME_BYTES:], mono(0) * 16)
        self.assertLess(len(audio), len(pre_roll) + metadata['captureMs'] * 32)
        self.assertEqual(len(model.inputs), (metadata['captureMs'] // 20 * 320) // 512)

    def test_wake_prefix_audio_is_unchanged(self):
        session = self.start_wake(early_wake=True)
        expected = bytes(session.prefix) + mono(2100) * 15
        for _ in range(15): messages = session.accept_stereo(frame(2100))
        self.assertEqual(messages[0]['purpose'], 'wake_prefix')
        self.assertEqual(messages[1], expected)

    def test_command_captured_during_confirmation_has_trimmed_tail(self):
        session = self.start_wake()
        for _ in range(60): messages = session.accept_stereo(frame(0))
        self.assertEqual(messages[0]['purpose'], 'wake_and_command')
        for _ in range(50): session.accept_stereo(frame(3000))
        for _ in range(50): session.accept_stereo(frame(0))
        metadata, audio = session.control({'type': 'wake.confirmed',
            'sessionId': session.session_id, 'followup': True})
        self.assertEqual(metadata['purpose'], 'command')
        self.assertEqual(metadata['captureMs'], 2000)
        self.assertEqual(metadata['sentMs'], 1320)
        self.assertEqual(audio, mono(3000) * 50 + mono(0) * 16)
