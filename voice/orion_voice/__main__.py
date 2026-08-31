"""Command-line entry point for Orion's local voice workers."""

from __future__ import annotations

import argparse
from pathlib import Path

from .listener import DEFAULT_COMMAND_TIMEOUT_SECONDS, VoiceLoopController, VoiceLoopWorker
from .speech import (
    DEFAULT_ASR_MODEL_DIRECTORY,
    DEFAULT_VAD_MODEL_PATH,
    MoonshineTranscriber,
    SherpaSpeechSegmenter,
)
from .tts import PiperSynthesizer
from .wake import (
    DEFAULT_CAPTURE_DEVICE,
    DEFAULT_WAKE_MODEL_DIRECTORY,
    DEFAULT_WAKE_SOCKET_PATH,
    AlsaPcmCapture,
    SherpaWakeDetector,
    WakeEventPublisher,
    WakeWorker,
    wait_for_command,
    wait_for_wake,
)
from .worker import DEFAULT_TTS_OUTPUT_DIRECTORY, DEFAULT_TTS_SOCKET_PATH, TtsWorker


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="orion-voice")
    commands = parser.add_subparsers(dest="command", required=True)

    worker = commands.add_parser("tts-worker", help="run the persistent Piper worker")
    worker.add_argument("--socket", type=Path, default=DEFAULT_TTS_SOCKET_PATH)
    worker.add_argument(
        "--output-dir", type=Path, default=DEFAULT_TTS_OUTPUT_DIRECTORY
    )

    wake = commands.add_parser(
        "wake-worker", help="listen locally for Orion's configured wake phrase"
    )
    wake.add_argument("--device", default=DEFAULT_CAPTURE_DEVICE)
    wake.add_argument("--model-dir", type=Path, default=DEFAULT_WAKE_MODEL_DIRECTORY)
    wake.add_argument("--socket", type=Path, default=DEFAULT_WAKE_SOCKET_PATH)
    wake.add_argument("--threads", type=int, default=2)
    wake.add_argument("--score", type=float, default=3.0)
    wake.add_argument("--threshold", type=float, default=0.10)

    wait = commands.add_parser(
        "wait-wake", help="wait for one event from the wake-word worker"
    )
    wait.add_argument("--socket", type=Path, default=DEFAULT_WAKE_SOCKET_PATH)

    listener = commands.add_parser(
        "listen-worker",
        help="run wake detection, command capture, and local transcription",
    )
    listener.add_argument("--device", default=DEFAULT_CAPTURE_DEVICE)
    listener.add_argument(
        "--wake-model-dir", type=Path, default=DEFAULT_WAKE_MODEL_DIRECTORY
    )
    listener.add_argument(
        "--asr-model-dir", type=Path, default=DEFAULT_ASR_MODEL_DIRECTORY
    )
    listener.add_argument("--vad-model", type=Path, default=DEFAULT_VAD_MODEL_PATH)
    listener.add_argument("--socket", type=Path, default=DEFAULT_WAKE_SOCKET_PATH)
    listener.add_argument("--threads", type=int, default=2)
    listener.add_argument("--score", type=float, default=3.0)
    listener.add_argument("--threshold", type=float, default=0.10)
    listener.add_argument("--vad-threshold", type=float, default=0.5)
    listener.add_argument("--silence-seconds", type=float, default=0.8)
    listener.add_argument(
        "--command-timeout",
        type=float,
        default=DEFAULT_COMMAND_TIMEOUT_SECONDS,
    )

    command = commands.add_parser(
        "wait-command", help="wait for one terminal command-capture result"
    )
    command.add_argument("--socket", type=Path, default=DEFAULT_WAKE_SOCKET_PATH)
    return parser


def main() -> int:
    arguments = build_parser().parse_args()
    if arguments.command == "wait-wake":
        print(wait_for_wake(arguments.socket).to_json_line().decode().rstrip())
        return 0
    if arguments.command == "wait-command":
        event = wait_for_command(arguments.socket)
        print(event.to_json_line().decode().rstrip())
        if event.state == "transcribed":
            return 0
        return 2 if event.state == "timed_out" else 1
    if arguments.command == "wake-worker":
        detector = SherpaWakeDetector(
            arguments.model_dir,
            num_threads=arguments.threads,
            keywords_score=arguments.score,
            keywords_threshold=arguments.threshold,
        )
        wake_worker = WakeWorker(
            detector,
            AlsaPcmCapture(arguments.device),
            WakeEventPublisher(arguments.socket),
        )
        try:
            wake_worker.serve_forever()
        except KeyboardInterrupt:
            wake_worker.close()
        return 0
    if arguments.command == "listen-worker":
        print("orion-listener: loading local voice models", flush=True)
        detector = SherpaWakeDetector(
            arguments.wake_model_dir,
            num_threads=arguments.threads,
            keywords_score=arguments.score,
            keywords_threshold=arguments.threshold,
        )
        segmenter = SherpaSpeechSegmenter(
            arguments.vad_model,
            threshold=arguments.vad_threshold,
            min_silence_seconds=arguments.silence_seconds,
        )
        transcriber = MoonshineTranscriber(
            arguments.asr_model_dir,
            num_threads=arguments.threads,
        )
        publisher = WakeEventPublisher(arguments.socket)
        controller = VoiceLoopController(
            detector,
            segmenter,
            transcriber,
            publisher,
            command_timeout_seconds=arguments.command_timeout,
        )
        listener_worker = VoiceLoopWorker(
            controller,
            AlsaPcmCapture(arguments.device),
            publisher,
            detector.configured_phrase,
        )
        try:
            listener_worker.serve_forever()
        except KeyboardInterrupt:
            listener_worker.close()
        return 0

    synthesizer = PiperSynthesizer()
    worker = TtsWorker(synthesizer, arguments.socket, arguments.output_dir)
    try:
        worker.serve_forever()
    except KeyboardInterrupt:
        worker.stop()
        worker.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
