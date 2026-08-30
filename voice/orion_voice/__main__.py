"""Command-line entry point for Orion's local voice workers."""

from __future__ import annotations

import argparse
from pathlib import Path

from .tts import ChatterboxNanoSynthesizer, benchmark
from .worker import DEFAULT_TTS_OUTPUT_DIRECTORY, DEFAULT_TTS_SOCKET_PATH, TtsWorker


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="orion-voice")
    commands = parser.add_subparsers(dest="command", required=True)

    worker = commands.add_parser("tts-worker", help="run the persistent Nano worker")
    worker.add_argument("--socket", type=Path, default=DEFAULT_TTS_SOCKET_PATH)
    worker.add_argument("--output-dir", type=Path, default=DEFAULT_TTS_OUTPUT_DIRECTORY)

    benchmark_parser = commands.add_parser(
        "benchmark-tts", help="benchmark Chatterbox Nano on this computer"
    )
    benchmark_parser.add_argument(
        "--text", default="Hello. I am Orion, and my voice is running locally."
    )
    benchmark_parser.add_argument("--output-dir", type=Path, default=Path("/tmp/orion-tts-benchmark"))
    benchmark_parser.add_argument("--iterations", type=int, default=3)
    return parser


def main() -> int:
    arguments = build_parser().parse_args()
    if arguments.command == "benchmark-tts":
        benchmark(arguments.text, arguments.output_dir, arguments.iterations)
        return 0

    synthesizer = ChatterboxNanoSynthesizer(device="cpu")
    worker = TtsWorker(synthesizer, arguments.socket, arguments.output_dir)
    try:
        worker.serve_forever()
    except KeyboardInterrupt:
        worker.stop()
        worker.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
