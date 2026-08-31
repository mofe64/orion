"""Command-line entry point for Orion's local voice workers."""

from __future__ import annotations

import argparse
from pathlib import Path

from .tts import PiperSynthesizer
from .wake import (
    DEFAULT_CAPTURE_DEVICE,
    DEFAULT_WAKE_MODEL_DIRECTORY,
    DEFAULT_WAKE_SOCKET_PATH,
    AlsaPcmCapture,
    SherpaWakeDetector,
    WakeEventPublisher,
    WakeWorker,
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
    return parser


def main() -> int:
    arguments = build_parser().parse_args()
    if arguments.command == "wait-wake":
        print(wait_for_wake(arguments.socket).to_json_line().decode().rstrip())
        return 0
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
