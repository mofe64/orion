from pathlib import Path
import struct
import tempfile
import unittest
import wave

from orion_voice.tts import ORION_PLAYBACK_SAMPLE_RATE, PiperSynthesizer


class FakeChunk:
    sample_rate = 22_050
    sample_width = 2
    sample_channels = 1
    audio_int16_bytes = struct.pack("<hhhh", 0, 1_000, -1_000, 0)


class FakeVoice:
    def synthesize(self, text: str):
        if text:
            yield FakeChunk()


class EmptyVoice:
    def synthesize(self, text: str):
        return iter(())


class PiperSynthesizerTests(unittest.TestCase):
    def test_loads_model_and_writes_orion_playback_format(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            model_path = root / "test.onnx"
            model_path.write_bytes(b"model")
            Path(f"{model_path}.json").write_text("{}")
            load_arguments = {}

            def load_voice(path, **arguments):
                load_arguments["path"] = path
                load_arguments.update(arguments)
                return FakeVoice()

            synthesizer = PiperSynthesizer(model_path, voice_loader=load_voice)
            output_path = root / "speech.wav"
            duration = synthesizer.synthesize("Hello.", output_path)

            self.assertEqual(load_arguments["path"], model_path)
            self.assertEqual(load_arguments["config_path"], Path(f"{model_path}.json"))
            self.assertFalse(load_arguments["use_cuda"])
            with wave.open(str(output_path), "rb") as output:
                self.assertEqual(output.getframerate(), ORION_PLAYBACK_SAMPLE_RATE)
                self.assertEqual(output.getsampwidth(), 2)
                self.assertEqual(output.getnchannels(), 2)
                self.assertAlmostEqual(
                    duration,
                    output.getnframes() / ORION_PLAYBACK_SAMPLE_RATE,
                )
                left, right = struct.unpack("<hh", output.readframes(1))
                self.assertEqual(left, right)

    def test_reports_missing_model_before_importing_piper(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            missing = Path(directory) / "missing.onnx"
            with self.assertRaisesRegex(FileNotFoundError, "Piper voice model not found"):
                PiperSynthesizer(missing)

    def test_removes_empty_output(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            model_path = root / "test.onnx"
            model_path.write_bytes(b"model")
            Path(f"{model_path}.json").write_text("{}")
            synthesizer = PiperSynthesizer(
                model_path,
                voice_loader=lambda *args, **kwargs: EmptyVoice(),
            )
            output_path = root / "empty.wav"

            with self.assertRaisesRegex(RuntimeError, "generated no audio"):
                synthesizer.synthesize("Hello.", output_path)
            self.assertFalse(output_path.exists())


if __name__ == "__main__":
    unittest.main()
