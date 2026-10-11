#!/usr/bin/env python3
"""Time a fixed local WAV through an isolated production Qwen ASR adapter."""
from __future__ import annotations

import argparse
from contextlib import contextmanager, redirect_stdout
import hashlib
import io
import json
import math
import os
from pathlib import Path
import statistics
import sys
import time
import wave


SPEECH_ROOT = Path(__file__).resolve().parents[2] / "speech"
sys.path.insert(0, str(SPEECH_ROOT))
from orion_speech_worker.pi import QwenGgufTranscriber


def read_fixture(path: Path) -> tuple[bytes, dict]:
    # Bound both file reading and inference to the speech protocol's 33 s cap.
    with path.open("rb") as source:
        data = source.read(2 * 1024 * 1024 + 1)
    if len(data) > 2 * 1024 * 1024:
        raise ValueError("WAV fixture exceeds 2 MiB")
    with wave.open(io.BytesIO(data), "rb") as audio:
        if (audio.getnchannels(), audio.getsampwidth(), audio.getframerate(),
                audio.getcomptype()) != (1, 2, 16000, "NONE"):
            raise ValueError("WAV must be uncompressed PCM16, 16 kHz, mono")
        frames = audio.getnframes()
        if not 0 < frames <= 33 * 16000:
            raise ValueError("WAV duration must be greater than zero and at most 33 seconds")
        pcm = audio.readframes(frames)
        if len(pcm) != frames * 2:
            raise ValueError("WAV is truncated")
    return pcm, {"sha256": hashlib.sha256(data).hexdigest(),
                 "duration_ms": frames / 16, "sample_rate": 16000}


@contextmanager
def isolated_environment(binary: Path):
    # The adapter's native launcher is a child Python module. Point it at this
    # checkout as well, even when the caller has no speech package installed.
    keys = ("ORION_LLAMA_SERVER", "PYTHONPATH", "ORION_ASR_CAPTURE_DIR")
    previous = {key: os.environ.get(key) for key in keys}
    os.environ["ORION_LLAMA_SERVER"] = str(binary)
    os.environ["PYTHONPATH"] = os.pathsep.join(filter(None, (
        str(SPEECH_ROOT), previous["PYTHONPATH"])))
    # A diagnostic capture setting must not create duplicate audio fixtures.
    os.environ.pop("ORION_ASR_CAPTURE_DIR", None)
    try:
        yield
    finally:
        for key, value in previous.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value


def benchmark(pcm, model, binary, runs, *, factory=QwenGgufTranscriber,
              clock=time.perf_counter):
    if not 1 <= runs <= 1000:
        raise ValueError("runs must be between 1 and 1000")
    with isolated_environment(binary), redirect_stdout(sys.stderr):
        transcriber = factory(model)
        try:
            transcriber.transcribe(pcm)  # Exactly one untimed warm-up.
            samples = []
            for _ in range(runs):
                started = clock()
                transcriber.transcribe(pcm)
                samples.append((clock() - started) * 1000)
        finally:
            # The adapter owns only the process it started; no systemd calls,
            # live service endpoints, or process-name based cleanup are used.
            transcriber.close()
    return {"warmup_runs": 1, "timed_runs": runs,
            "median_ms": statistics.median(samples),
            "p95_ms": sorted(samples)[math.ceil(0.95 * runs) - 1],
            "p95_method": "nearest_rank", "samples_ms": samples}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wav", type=Path, required=True)
    parser.add_argument("--model", type=Path, default=os.environ.get("ORION_ASR_MODEL_DIR"),
                        help="Qwen folder containing model.gguf and mmproj.gguf")
    parser.add_argument("--llama-server", type=Path, default=os.environ.get("ORION_LLAMA_SERVER"))
    parser.add_argument("--runs", type=int, default=10)
    parser.add_argument("--label", default="trial", help="e.g. tracking-off or tracking-faces")
    args = parser.parse_args(argv)
    try:
        pcm, fixture = read_fixture(args.wav)
        if sys.platform != "linux":
            raise ValueError("Real Qwen timing requires Linux (the Pi native launcher)")
        if args.model is None or args.llama_server is None:
            raise ValueError("Provide --model and --llama-server or their ORION environment variables")
        model = args.model.resolve(strict=True)
        binary = args.llama_server.resolve(strict=True)
        result = benchmark(pcm, model, binary, args.runs)
        result.update(ok=True, label=args.label, fixture=fixture, model=str(model),
                      llama_server=str(binary), threads=int(os.environ.get("ORION_ASR_THREADS", "3")))
    except (Exception, KeyboardInterrupt) as error:
        print(json.dumps({"ok": False, "error": str(error) or type(error).__name__}))
        return 1
    print(json.dumps(result, allow_nan=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
