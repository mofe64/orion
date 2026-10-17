# Orion speech worker

Separate Python workers run Qwen3-ASR transcription and Piper Alba Medium TTS on the Pi.
The [Rust coordinator](../coordinator/README.md) submits inference jobs and owns
the surrounding voice session, agent call and playback lifecycle.

## Setup on the Pi

Use 64-bit Linux on the Pi 5 with 8 GB RAM. The speech environment uses Python
3.12, the pinned Sherpa ONNX/Piper model, and the native
Qwen GGUF server. The listener uses a separate Python 3.12 environment with
Silero ONNX and Rustpotter.
See [Pi installation](../docs/quickstart.md#prepare-a-new-pi).

`ORION_LLAMA_SERVER` selects the Qwen server program; the ASR model folder contains
`model.gguf` and `mmproj.gguf`.
The Pi adapter pre-fills Qwen's English transcription prefix, so the model
decodes English speech without first guessing among its supported languages.
Orion's agent speaks English unless asked explicitly to switch languages.
The private Qwen HTTP endpoint binds a random loopback port and requires a
per-process key. Linux kills that child if its Python owner exits.

`piper-alba-medium` uses the single British English Alba voice and three CPU
threads by default. The installer verifies its archive and weights, then stores
them under `~/.local/share/orion/voice-stack/models/piper-alba-medium/`.
`ORION_PIPER_MODEL_DIR` points the worker to that folder. Piper generates 22,050 Hz
audio; the worker resamples each completed sentence to Orion's 24,000 Hz protocol
and divides it into chunks of at most two seconds.
The [Piper Alba model card](https://huggingface.co/rhasspy/piper-voices/blob/main/en/en_GB/alba/medium/MODEL_CARD)
records the voice's training source and dataset license.

Service inference uses the prepared local model files.

## Inference protocol

The coordinator launches `python -I -m orion_speech_worker.worker` with private
stdin/stdout pipes. Its first JSON line contains `protocol: 2`, `role` (`asr` or `tts`), `asr_model`, and
`tts_model`. Each worker loads only its assigned model and returns `ready`.
Protocol 1 remains available for legacy development clients.
These local inference jobs carry audio or text, model settings and job IDs.
Library output is redirected to stderr so it cannot corrupt protocol framing.

Each job has a positive integer `id` and a `method`:

- `transcribe`: a `bytes` count followed immediately by that many PCM16 bytes
  at 16 kHz, mono. The reply is `transcript` with `text` and `language`.
- `synthesize`: a `text` string. Piper uses its fixed Alba voice. Replies are
  `chunk` JSON metadata followed by
  mono 24 kHz PCM16, then an explicit `end` with the next sequence number.

Metadata lines are bounded to 64 KiB. Input audio is bounded to 33 seconds;
output chunks to two seconds. Replies have no total audio-duration cap. The
coordinator splits text into at most 160 Unicode characters per inference job,
which retains its 240-second deadline; Piper also splits native inputs and rejects
more than 120 seconds of audio from any one input. Failed jobs
send `error` and end the worker. The coordinator checks IDs, framing, rates,
lengths, sequences, and completion before accepting results.

Jobs run serially within each worker. The coordinator can submit final-answer
sentences as they arrive. Cancelling ASR or TTS retires only that worker; the
next job reloads its model. Completed jobs reuse loaded models. Changing speech
settings restarts the coordinator and workers.

## Validation

From the repository root:

```bash
PYTHONPATH=speech speech/.venv/bin/python -m unittest discover -s speech/tests -v
cargo test --manifest-path coordinator/Cargo.toml
```

Python tests check inference framing and TTS conversion without loading models.
Coordinator tests cover voice orchestration with fake inference and Pi peers.
Neither establishes physical echo quality or actual inference latency.
