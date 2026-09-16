# Orion speech worker

Separate Python workers run Qwen3-ASR transcription and Pocket TTS on the Pi.
The Apple Silicon adapter retains Qwen3-ASR and Chatterbox support.
It has no agent client, Pi transport, wake confirmation, playback control, or
listening-window state. The [Rust coordinator](../coordinator/README.md) owns
those responsibilities.

## Setup on Apple Silicon

The workstation adapter requires Apple Silicon and Python 3.12.
From this directory:

```bash
uv sync --python 3.12 --locked
.venv/bin/orion-voice-models
```

Weights live in the Hugging Face cache, not this directory.
After moving an existing checkout, run `uv sync` here to refresh editable-package
paths and console scripts. This standalone adapter is for development; Studio
uses the Pi service and does not launch workstation speech workers.

## Setup on the Pi

Use 64-bit Linux on the Pi 5 with 8 GB RAM. The speech environment uses Python
3.11, the CPU Torch wheel, Pocket TTS, and the native Qwen GGUF server. The
listener uses a separate Python 3.12 environment with Silero ONNX and Rustpotter.
See [Pi installation](../docs/quickstart.md#pi-local-voice-and-agent).

`ORION_SPEECH_BACKEND=pi` selects the CPU adapters. `ORION_LLAMA_SERVER` selects
the native binary; the ASR model folder contains `model.gguf` and `mmproj.gguf`.
The private Qwen HTTP endpoint binds a random loopback port and requires a
per-process key. Linux kills that child if its Python owner exits.

Pocket supports `pocket-fp32` and `pocket-int8`. The presets are Anna, Azelma,
Cosette, Eve, Fantine, Jane, Vera, and Alba. Conditioning is cached per voice.
All preset assets are downloaded before activation; service inference uses the
local cache with `HF_HUB_OFFLINE=1`.

## Inference protocol

The coordinator launches `python -m orion_speech_worker.worker` with private
stdin/stdout pipes. Its first JSON line contains `protocol: 2`, `role` (`asr` or `tts`), `asr_model`, and
`tts_model`. Each worker loads only its assigned model and returns `ready`.
Protocol 1 remains available for legacy development clients.
No credentials are needed or passed. Library output is redirected to stderr.

Each job has a positive integer `id` and a `method`:

- `transcribe`: a `bytes` count followed immediately by that many PCM16 bytes
  at 16 kHz, mono. The reply is `transcript` with `text` and `language`.
- `synthesize`: a `text` string and a `voice` preset for Pocket. Replies are `chunk` JSON metadata followed by
  mono 24 kHz PCM16, then an explicit `end` with the next sequence number.

Metadata lines are bounded to 64 KiB. Input audio is bounded to 33 seconds;
output chunks to two seconds and complete replies to 120 seconds. Failed jobs
send `error` and end the worker. The coordinator checks IDs, framing, rates,
lengths, sequences, and completion before accepting results.

Jobs run serially within each worker. The coordinator can submit final-answer
sentences as they arrive. Cancelling ASR or TTS retires only that worker; the
next job reloads its model. Completed jobs reuse loaded models. Changing the
voice preset does not restart either worker.

## Validation

From the repository root:

```bash
PYTHONPATH=speech speech/.venv/bin/python -m unittest discover -s speech/tests -v
cargo test --manifest-path coordinator/Cargo.toml
```

Python tests cover model-independent inference framing and TTS conversion.
Coordinator tests cover voice orchestration with fake inference and Pi peers.
Neither establishes physical echo quality or actual inference latency.
