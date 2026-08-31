from pathlib import Path
import tempfile
import unittest

import numpy as np

from orion_voice.speech import (
    SPEECH_SAMPLE_RATE,
    MoonshineTranscriber,
    SherpaSpeechSegmenter,
)


ASR_MODEL_FILES = [
    "preprocess.onnx",
    "encode.int8.onnx",
    "uncached_decode.int8.onnx",
    "cached_decode.int8.onnx",
    "tokens.txt",
]


class FakeResult:
    text = "  Return home.  "


class FakeAsrStream:
    def __init__(self) -> None:
        self.accepted = []
        self.result = FakeResult()

    def accept_waveform(self, sample_rate, samples) -> None:
        self.accepted.append((sample_rate, samples.copy()))


class FakeRecognizer:
    def __init__(self) -> None:
        self.stream = FakeAsrStream()
        self.decode_count = 0

    def create_stream(self):
        return self.stream

    def decode_stream(self, stream) -> None:
        self.decode_count += 1


class FakeSegment:
    def __init__(self, samples) -> None:
        self.samples = samples


class FakeVad:
    def __init__(self) -> None:
        self.accepted = []
        self.segments = []
        self.reset_count = 0

    def accept_waveform(self, samples) -> None:
        self.accepted.append(samples.copy())
        if len(self.accepted) == 2:
            self.segments.append(FakeSegment(np.array([0.25, -0.5], np.float32)))

    def empty(self) -> bool:
        return not self.segments

    @property
    def front(self):
        return self.segments[0]

    def pop(self) -> None:
        self.segments.pop(0)

    def reset(self) -> None:
        self.reset_count += 1
        self.segments.clear()


def write_asr_model(directory: Path) -> None:
    for filename in ASR_MODEL_FILES:
        (directory / filename).write_bytes(b"test")


class MoonshineTranscriberTests(unittest.TestCase):
    def test_transcribes_in_memory_audio_and_reports_timings(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            model_directory = Path(directory)
            write_asr_model(model_directory)
            recognizer = FakeRecognizer()
            factory_arguments = {}
            times = iter([10.0, 10.25])

            def factory(**arguments):
                factory_arguments.update(arguments)
                return recognizer

            transcriber = MoonshineTranscriber(
                model_directory,
                num_threads=3,
                recognizer_factory=factory,
                monotonic=lambda: next(times),
            )
            samples = np.zeros(SPEECH_SAMPLE_RATE * 2, dtype=np.float32)
            result = transcriber.transcribe(samples)

            self.assertEqual(result.text, "Return home.")
            self.assertEqual(result.audio_seconds, 2.0)
            self.assertEqual(result.inference_seconds, 0.25)
            self.assertEqual(recognizer.decode_count, 1)
            self.assertEqual(recognizer.stream.accepted[0][0], SPEECH_SAMPLE_RATE)
            self.assertEqual(factory_arguments["num_threads"], 3)
            self.assertTrue(factory_arguments["encoder"].endswith("int8.onnx"))

    def test_rejects_missing_model_and_empty_audio(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(FileNotFoundError, "install-models"):
                MoonshineTranscriber(Path(directory), recognizer_factory=lambda **_: None)

        with tempfile.TemporaryDirectory() as directory:
            model_directory = Path(directory)
            write_asr_model(model_directory)
            transcriber = MoonshineTranscriber(
                model_directory,
                recognizer_factory=lambda **_: FakeRecognizer(),
            )
            with self.assertRaisesRegex(ValueError, "empty utterance"):
                transcriber.transcribe([])


class SherpaSpeechSegmenterTests(unittest.TestCase):
    def test_buffers_windows_emits_segments_and_resets(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            model_path = Path(directory) / "silero_vad.onnx"
            model_path.write_bytes(b"test")
            fake_vad = FakeVad()
            factory_arguments = {}

            def factory(**arguments):
                factory_arguments.update(arguments)
                return fake_vad, 4

            segmenter = SherpaSpeechSegmenter(
                model_path,
                threshold=0.4,
                vad_factory=factory,
            )
            pcm = np.array([0, 8192, -16384, 32767] * 2, dtype="<i2").tobytes()
            segments = segmenter.accept_pcm16(pcm)

            self.assertEqual(len(fake_vad.accepted), 2)
            self.assertEqual(len(segments), 1)
            np.testing.assert_allclose(segments[0], [0.25, -0.5])
            self.assertEqual(factory_arguments["threshold"], 0.4)

            segmenter.reset()
            self.assertEqual(fake_vad.reset_count, 1)

    def test_rejects_missing_model_and_partial_pcm(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(FileNotFoundError, "Silero VAD"):
                SherpaSpeechSegmenter(
                    Path(directory) / "missing.onnx",
                    vad_factory=lambda **_: (FakeVad(), 4),
                )

        with tempfile.TemporaryDirectory() as directory:
            model_path = Path(directory) / "silero_vad.onnx"
            model_path.write_bytes(b"test")
            segmenter = SherpaSpeechSegmenter(
                model_path,
                vad_factory=lambda **_: (FakeVad(), 4),
            )
            with self.assertRaisesRegex(ValueError, "incomplete"):
                segmenter.accept_pcm16(b"\x00")


if __name__ == "__main__":
    unittest.main()
