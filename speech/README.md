# Orion speech worker

The Python worker runs Qwen3-ASR transcription and Chatterbox speech synthesis.
It has no agent client, Pi transport, wake confirmation, playback control, or
listening-window state. The [Rust coordinator](../coordinator/README.md) owns
those responsibilities.

## Setup on Apple Silicon

The commissioned inference stack requires Apple Silicon and Python 3.12.
From this directory:

```bash
uv sync --python 3.12 --locked
.venv/bin/orion-voice-models
```

Weights live in the Hugging Face cache, not this directory. See
[model management](../docs/how-to/manage-studio-voice-models.md).
After moving an existing checkout, run `uv sync` here to refresh editable-package
paths and console scripts. Studio defaults to `speech/.venv/bin/python`.

## Inference protocol

The coordinator launches `python -m orion_speech_worker.worker` with private
stdin/stdout pipes. Its first JSON line contains `protocol: 1`, `asr_model`, and
`tts_model`; the worker loads models and returns `ready` with their metadata.
No credentials are needed or passed. Library output is redirected to stderr.

Each job has a positive integer `id` and a `method`:

- `transcribe`: a `bytes` count followed immediately by that many PCM16 bytes
  at 16 kHz, mono. The reply is `transcript` with `text` and `language`.
- `synthesize`: a `text` string. Replies are `chunk` JSON metadata followed by
  mono 24 kHz PCM16, then an explicit `end` with the next sequence number.

Metadata lines are bounded to 64 KiB. Input audio is bounded to 18 seconds;
output chunks to two seconds and complete replies to 120 seconds. Failed jobs
send `error` and end the worker. The coordinator checks IDs, framing, rates,
lengths, sequences, and completion before accepting results.

Jobs run serially. Sentence segmentation and gain handling remain inside the
TTS implementation. Cancelling active inference retires the process; the next
job reloads its models. Completed jobs reuse loaded models.

## Validation

From the repository root:

```bash
PYTHONPATH=speech speech/.venv/bin/python -m unittest discover -s speech/tests -v
cargo test --manifest-path coordinator/Cargo.toml
```

Python tests cover model-independent inference framing and TTS conversion.
Coordinator tests cover voice orchestration with fake inference and Pi peers.
Neither establishes physical echo quality or actual inference latency.
