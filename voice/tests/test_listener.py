import unittest

import numpy as np

from orion_voice.listener import VoiceLoopController
from orion_voice.speech import Transcription


class FakeClock:
    def __init__(self) -> None:
        self.now = 0.0

    def __call__(self) -> float:
        return self.now


class FakeDetector:
    configured_phrase = "HELLO WORLD"

    def __init__(self) -> None:
        self.results = []

    def accept_pcm16(self, pcm: bytes):
        return self.results.pop(0) if self.results else []


class FakeSegmenter:
    def __init__(self) -> None:
        self.results = []
        self.error = None
        self.reset_count = 0

    def accept_pcm16(self, pcm: bytes):
        if self.error is not None:
            raise self.error
        return self.results.pop(0) if self.results else []

    def reset(self) -> None:
        self.reset_count += 1


class FakeTranscriber:
    def __init__(self, result=None, error=None) -> None:
        self.result = result or Transcription("Return home.", 1.25, 0.2)
        self.error = error
        self.samples = []

    def transcribe(self, samples):
        self.samples.append(samples)
        if self.error is not None:
            raise self.error
        return self.result


class FakePublisher:
    def __init__(self) -> None:
        self.events = []

    def publish(self, phrase: str) -> None:
        self.events.append(("wake_word", phrase))

    def publish_command(self, state: str, **details) -> None:
        self.events.append(("command", state, details))


class VoiceLoopControllerTests(unittest.TestCase):
    def build_controller(self, transcriber=None):
        detector = FakeDetector()
        segmenter = FakeSegmenter()
        transcriber = transcriber or FakeTranscriber()
        publisher = FakePublisher()
        clock = FakeClock()
        controller = VoiceLoopController(
            detector,
            segmenter,
            transcriber,
            publisher,
            command_timeout_seconds=5.0,
            monotonic=clock,
        )
        return controller, detector, segmenter, transcriber, publisher, clock

    def test_wake_then_transcript_then_rearms(self) -> None:
        controller, detector, segmenter, transcriber, publisher, _ = (
            self.build_controller()
        )
        detector.results.append(["HELLO WORLD"])
        utterance = np.array([0.25, -0.5], dtype=np.float32)
        segmenter.results.append([utterance])

        controller.process_pcm(b"wake")
        self.assertEqual(controller.state, "command_listening")
        controller.process_pcm(b"command")

        self.assertEqual(controller.state, "wake_listening")
        self.assertEqual(segmenter.reset_count, 2)
        np.testing.assert_array_equal(transcriber.samples[0], utterance)
        self.assertEqual(publisher.events[0], ("wake_word", "HELLO WORLD"))
        self.assertEqual(publisher.events[1][0:2], ("command", "transcribed"))
        self.assertEqual(publisher.events[1][2]["text"], "Return home.")

    def test_command_timeout_is_deterministic_and_rearms(self) -> None:
        controller, detector, segmenter, _, publisher, clock = (
            self.build_controller()
        )
        detector.results.append(["HELLO WORLD"])
        controller.process_pcm(b"wake")

        clock.now = 4.9
        controller.process_pcm(b"silence")
        self.assertEqual(controller.state, "command_listening")

        clock.now = 5.0
        controller.process_pcm(b"silence")
        self.assertEqual(controller.state, "wake_listening")
        self.assertEqual(publisher.events[-1][0:2], ("command", "timed_out"))
        self.assertEqual(segmenter.reset_count, 2)

    def test_transcription_failure_is_published_and_rearms(self) -> None:
        transcriber = FakeTranscriber(error=RuntimeError("decode failed"))
        controller, detector, segmenter, _, publisher, _ = self.build_controller(
            transcriber
        )
        detector.results.append(["HELLO WORLD"])
        segmenter.results.append([np.zeros(16, dtype=np.float32)])

        controller.process_pcm(b"wake")
        controller.process_pcm(b"command")

        self.assertEqual(controller.state, "wake_listening")
        self.assertEqual(publisher.events[-1][0:2], ("command", "failed"))
        self.assertEqual(publisher.events[-1][2]["error"], "decode failed")

    def test_segmentation_failure_is_published_and_rearms(self) -> None:
        controller, detector, segmenter, _, publisher, _ = self.build_controller()
        detector.results.append(["HELLO WORLD"])
        segmenter.error = RuntimeError("VAD failed")

        controller.process_pcm(b"wake")
        controller.process_pcm(b"command")

        self.assertEqual(controller.state, "wake_listening")
        self.assertEqual(publisher.events[-1][0:2], ("command", "failed"))
        self.assertEqual(publisher.events[-1][2]["error"], "VAD failed")


if __name__ == "__main__":
    unittest.main()
