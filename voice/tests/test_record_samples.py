import argparse
import json
from pathlib import Path
import tempfile
import unittest
import wave
from unittest.mock import patch

from orion_voice.record_samples import Manifest, microphone, sets, speaker_name


class RecorderTests(unittest.TestCase):
    def test_speaker_names_cannot_escape_the_output_directory(self):
        for value in ("mofe", "Alice2", "ROOM"):
            self.assertEqual(speaker_name(value), value)
        for value in ("", "../mofe", "a/b", "a b", "a-b", "é", "."):
            with self.assertRaises(argparse.ArgumentTypeError):
                speaker_name(value)

    def test_manifest_resumes_kept_slots_and_never_overwrites_redos_or_orphans(self):
        with tempfile.TemporaryDirectory() as folder:
            manifest = Manifest(folder, "mofe")
            condition = sets()[0]
            pcm = b"\x00\x10" * 320
            first = manifest.add(condition, 1, "Hey Orion", pcm)
            self.assertFalse(manifest.complete("normal", 1))
            first["status"] = "redone"
            manifest.save()
            (Path(folder) / "normal-002.wav").write_bytes(b"orphan")
            second = manifest.add(condition, 1, "Hey Orion", pcm)
            second["status"] = "kept"
            manifest.save()
            resumed = Manifest(folder, "mofe")
            self.assertTrue(resumed.complete("normal", 1))
            self.assertFalse(resumed.complete("normal", 2))
            self.assertEqual(second["file"], "normal-003.wav")
            self.assertEqual((Path(folder) / "normal-002.wav").read_bytes(), b"orphan")
            self.assertEqual(json.loads(resumed.path.read_text())["takes"][0]["status"], "redone")
            self.assertAlmostEqual(second["peak_dbfs"], -18.06, places=2)
            with wave.open(str(Path(folder) / first["file"])) as audio:
                self.assertEqual((audio.getnchannels(), audio.getframerate(), audio.getsampwidth()), (1, 16000, 2))
                self.assertEqual(audio.readframes(320), pcm)

    def test_listener_is_restored_after_capture_failure_or_interrupt(self):
        for error in (RuntimeError("capture"), KeyboardInterrupt()):
            with patch("orion_voice.record_samples.subprocess.run") as run:
                with self.assertRaises(type(error)):
                    with microphone({"hardware": "v2"}):
                        raise error
                self.assertEqual(run.call_args.args[0], ["sudo", "systemctl", "start", "orion-listener"])

    def test_prompt_counts_and_command_lengths(self):
        plan = sets()
        self.assertEqual([len(s["prompts"]) for s in plan], [10] * 11 + [20] + [3] * 12)
        self.assertEqual(plan[11]["seconds"], 6)
        self.assertEqual(len(set(plan[11]["prompts"])), 20)
