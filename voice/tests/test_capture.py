from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from orion_voice.capture import (AlsaPcmCapture, DEFAULT_CAPTURE_CARD,
                                 DEFAULT_CAPTURE_CONFIGURATOR, DEFAULT_CAPTURE_DEVICE)


class AlsaPcmCaptureTests(unittest.TestCase):
    def test_uses_stable_respeaker_capture_contract(self) -> None:
        capture = AlsaPcmCapture()
        command = capture.command()
        self.assertEqual(command[command.index("-D") + 1], DEFAULT_CAPTURE_DEVICE)
        self.assertEqual(command[command.index("-r") + 1], "16000")
        self.assertEqual(command[command.index("-c") + 1], "1")
        self.assertEqual(command[command.index("-f") + 1], "S16_LE")
        self.assertEqual(
            capture.configure_command(),
            [str(DEFAULT_CAPTURE_CONFIGURATOR), DEFAULT_CAPTURE_CARD],
        )

    def test_configures_mixer_before_opening_arecord(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            configurator = Path(directory) / "configure-capture.sh"
            configurator.write_text("#!/bin/sh\n")
            capture = AlsaPcmCapture(configurator=configurator)

            with (
                patch("orion_voice.capture.subprocess.run") as configure,
                patch("orion_voice.capture.subprocess.Popen") as popen,
            ):
                capture.open()

            configure.assert_called_once_with(
                [str(configurator), DEFAULT_CAPTURE_CARD],
                check=True,
                stdout=subprocess.DEVNULL,
            )
            popen.assert_called_once()


