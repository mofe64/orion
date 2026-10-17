# Orion speech workers

Separate Python workers run Qwen speech recognition and Piper speech synthesis. The coordinator owns voice sessions and submits inference jobs. Inference means running a model to produce an output.

## Setup on the Pi

The [release installer](../docs/quickstart.md) supplies Python 3.12, workers and pinned model files. `ORION_LLAMA_SERVER` selects the Qwen server executable. Its model directory contains `model.gguf` and `mmproj.gguf`.

Qwen receives an English transcription prefix. Its private HTTP endpoint uses a random loopback port and a process key. Linux terminates the server when its Python owner exits.

Piper uses the Alba Medium British English voice. `ORION_PIPER_MODEL_DIR` selects its directory. The worker resamples output to mono 24 kHz PCM16 and sends chunks of at most two seconds.

PCM16 represents each audio sample with a signed 16-bit integer. The [Alba model card](https://huggingface.co/rhasspy/piper-voices/blob/main/en/en_GB/alba/medium/MODEL_CARD) gives its license.

## Inference protocol

The coordinator launches the worker with private input and output pipes. The first JSON line selects protocol 2, worker role and model settings. The worker loads its assigned model before returning `ready`.

| Method | Input | Output |
| --- | --- | --- |
| `transcribe` | JSON byte count, then mono 16 kHz PCM16 | `transcript` with text and language |
| `synthesize` | JSON text | Ordered `chunk` messages with PCM16, then `end` |

Each job has a positive integer ID. Metadata lines have a 64 KiB limit. Recognition input has a 33-second limit. Each synthesis job has a 240-second deadline.

Jobs run serially within each worker. Completed jobs reuse loaded models. Cancellation retires the affected worker. The next job loads its model again. Model library output goes to stderr to protect protocol framing.

## Development

Run these steps from the repository root.

1. Install locked dependencies.

   ```bash
   uv sync --project speech --locked --managed-python
   ```

2. Run the worker tests.

   ```bash
   uv run --project speech --locked --managed-python python -m unittest discover -s speech/tests -v
   ```

See the [coordinator](../coordinator/README.md) for job ownership and [voice architecture](../docs/voice-architecture.md) for session flow.
