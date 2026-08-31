import json
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import unittest
from unittest.mock import patch

from orion_voice.wake import (
    DEFAULT_CAPTURE_CARD,
    DEFAULT_CAPTURE_CONFIGURATOR,
    DEFAULT_CAPTURE_DEVICE,
    WAKE_SAMPLE_RATE,
    AlsaPcmCapture,
    CommandEvent,
    SherpaWakeDetector,
    WakeEvent,
    WakeEventPublisher,
    wait_for_command,
    wait_for_wake,
)


MODEL_FILES = [
    "tokens.txt",
    "encoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx",
    "decoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx",
    "joiner-epoch-12-avg-2-chunk-16-left-64.int8.onnx",
    "orion_keywords.txt",
    "orion_keywords_raw.txt",
]


class FakeStream:
    def __init__(self) -> None:
        self.waveforms = []

    def accept_waveform(self, sample_rate, samples) -> None:
        self.waveforms.append((sample_rate, samples))


class FakeSpotter:
    def __init__(self) -> None:
        self.stream = FakeStream()
        self.ready = True
        self.reset_count = 0
        self.decode_count = 0

    def create_stream(self):
        return self.stream

    def is_ready(self, stream) -> bool:
        if self.ready:
            self.ready = False
            return True
        return False

    def decode_stream(self, stream) -> None:
        self.decode_count += 1

    def get_result(self, stream) -> str:
        return "HEY ORION"

    def reset_stream(self, stream) -> None:
        self.reset_count += 1


def write_fake_model(directory: Path) -> None:
    for filename in MODEL_FILES:
        contents = b"HELLO WORLD\n" if filename == "orion_keywords_raw.txt" else b"test"
        (directory / filename).write_bytes(contents)


class SherpaWakeDetectorTests(unittest.TestCase):
    def test_decodes_configured_keyword_and_resets_stream(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            model_directory = Path(directory)
            write_fake_model(model_directory)
            factory_arguments = {}
            fake_spotter = FakeSpotter()

            def factory(**arguments):
                factory_arguments.update(arguments)
                return fake_spotter

            detector = SherpaWakeDetector(
                model_directory,
                num_threads=3,
                keywords_score=1.7,
                keywords_threshold=0.3,
                spotter_factory=factory,
            )

            self.assertEqual(detector.accept_samples([0.0, 0.25]), ["HEY ORION"])
            self.assertEqual(detector.configured_phrase, "HELLO WORLD")
            self.assertEqual(fake_spotter.stream.waveforms[0][0], WAKE_SAMPLE_RATE)
            self.assertEqual(fake_spotter.decode_count, 1)
            self.assertEqual(fake_spotter.reset_count, 1)
            self.assertEqual(factory_arguments["num_threads"], 3)
            self.assertEqual(factory_arguments["keywords_score"], 1.7)
            self.assertEqual(factory_arguments["keywords_threshold"], 0.3)
            self.assertTrue(factory_arguments["encoder"].endswith("int8.onnx"))

    def test_reports_incomplete_model_before_importing_sherpa(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(FileNotFoundError, "install-models"):
                SherpaWakeDetector(Path(directory))

    def test_rejects_invalid_pcm_and_tuning(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            model_directory = Path(directory)
            write_fake_model(model_directory)
            detector = SherpaWakeDetector(
                model_directory,
                spotter_factory=lambda **arguments: FakeSpotter(),
            )
            with self.assertRaisesRegex(ValueError, "incomplete 16-bit sample"):
                detector.accept_pcm16(b"\x00")
            with self.assertRaisesRegex(ValueError, "between zero and one"):
                SherpaWakeDetector(
                    model_directory,
                    keywords_threshold=1.0,
                    spotter_factory=lambda **arguments: FakeSpotter(),
                )


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
                patch("orion_voice.wake.subprocess.run") as configure,
                patch("orion_voice.wake.subprocess.Popen") as popen,
            ):
                capture.open()

            configure.assert_called_once_with(
                [str(configurator), DEFAULT_CAPTURE_CARD],
                check=True,
                stdout=subprocess.DEVNULL,
            )
            popen.assert_called_once()


class WakeEventPublisherTests(unittest.TestCase):
    def test_publishes_ordered_event_and_cleans_socket(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            socket_path = Path(directory) / "wake.sock"
            publisher = WakeEventPublisher(socket_path)
            publisher.open()
            try:
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as subscriber:
                    subscriber.connect(str(socket_path))
                    publisher.accept_pending()
                    event = publisher.publish("HEY ORION")
                    received = json.loads(subscriber.makefile("rb").readline())

                self.assertEqual(event.event_id, 1)
                self.assertEqual(received["event"], "wake_word")
                self.assertEqual(received["phrase"], "HEY ORION")
            finally:
                publisher.close()
            self.assertFalse(socket_path.exists())

    def test_publishes_wake_and_command_with_ordered_ids(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            socket_path = Path(directory) / "wake.sock"
            publisher = WakeEventPublisher(socket_path)
            publisher.open()
            try:
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as subscriber:
                    subscriber.connect(str(socket_path))
                    publisher.accept_pending()
                    publisher.publish("HELLO WORLD")
                    publisher.publish_command(
                        "transcribed",
                        text="Return home.",
                        audio_seconds=1.2,
                        inference_seconds=0.3,
                    )
                    stream = subscriber.makefile("rb")
                    wake = json.loads(stream.readline())
                    command = json.loads(stream.readline())

                self.assertEqual(wake["event_id"], 1)
                self.assertEqual(command["event_id"], 2)
                self.assertEqual(command["state"], "transcribed")
                self.assertEqual(command["text"], "Return home.")
            finally:
                publisher.close()

    def test_wait_for_wake_reads_one_event(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            socket_path = Path(directory) / "wake.sock"
            listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            listener.bind(str(socket_path))
            listener.listen(1)

            def serve_event() -> None:
                connection, _ = listener.accept()
                with connection:
                    connection.sendall(
                        WakeEvent(9, "wake_word", "HEY ORION").to_json_line()
                    )

            server = threading.Thread(target=serve_event)
            server.start()
            try:
                event = wait_for_wake(socket_path)
                self.assertEqual(event, WakeEvent(9, "wake_word", "HEY ORION"))
            finally:
                server.join(timeout=1)
                listener.close()

    def test_wait_for_command_skips_wake_event(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            socket_path = Path(directory) / "wake.sock"
            listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            listener.bind(str(socket_path))
            listener.listen(1)

            def serve_events() -> None:
                connection, _ = listener.accept()
                with connection:
                    connection.sendall(
                        WakeEvent(1, "wake_word", "HELLO WORLD").to_json_line()
                    )
                    connection.sendall(
                        CommandEvent(
                            2,
                            "command",
                            "transcribed",
                            text="Return home.",
                            audio_seconds=1.2,
                            inference_seconds=0.3,
                        ).to_json_line()
                    )

            server = threading.Thread(target=serve_events)
            server.start()
            try:
                event = wait_for_command(socket_path)
                self.assertEqual(event.state, "transcribed")
                self.assertEqual(event.text, "Return home.")
            finally:
                server.join(timeout=1)
                listener.close()


if __name__ == "__main__":
    unittest.main()
