# Orion Studio processing worker

The worker receives Pi-triggered utterances, confirms the wake phrase with
Qwen3-ASR, extracts the command, invokes the configured agent and synthesizes
speech with Chatterbox. It never captures a workstation microphone or loads
Rustpotter. The Pi owns the sole wake detector.

## Setup on Apple Silicon

The deployed environment requires Apple Silicon and Python 3.12.
From this directory:

```bash
uv sync --python 3.12
.venv/bin/orion-voice-models
.venv/bin/python -m unittest discover -s tests -v
```

The model downloader prepares the Hugging Face cache. Studio startup does not
proactively download weights. See [model management](../../docs/how-to/manage-studio-voice-models.md).

Prepare the [Pi listener](../../voice/README.md), then launch Studio. No
certificate setup is required. Studio starts one worker child while the app is
open. Closing the Voice panel detaches its observer; quitting Studio stops the
worker. Connection credentials travel through the parent pipe and are not saved
in a service configuration. Reply model settings are saved separately.
The token and audio travel unencrypted on the local network.

## Processing contract

The authenticated loopback observer uses protocol version 7. Its hello contains
`type`, `protocol`, and `token`. Update Studio and the worker together.
The worker connects to Pi listener protocol version 1, confirms “Hey Orion”,
processes a buffered follow-up when needed and sends only extracted command text
to the agent. It owns uploads and Pi completion acknowledgments, with automatic
reconnect and cancellation of interrupted turns. The UI cannot claim playback
ownership. Microphone control is an explicit authenticated Pi operation.

The worker loads models once and serializes inference. A separate bounded
executor keeps HTTP uploads and playback polling from waiting behind model work.
Studio keeps one child per app instance; the Pi rejects competing processing
attachments. Qwen rejects unconfirmed wakes before invoking the agent. The Codex
provider cannot invoke movement or raw device operations.

Model-independent tests include real loopback WebSocket integration with fake
models; they do not establish physical microphone quality or inference latency.
See [voice architecture](../../docs/explanation/voice-architecture.md) for data
flow, deadlines, privacy and the separate gateway transport boundary.

Internally, replies stream as `speech.chunk` metadata followed by binary PCM16,
with ordered zero-based sequences and one `speech.end`. The worker
converts them to gateway WAV uploads; it reports completion only after synthesis
ends and Pi playback reaches a terminal result. Chatterbox gain is selected from the first audible
chunk and held for the reply to avoid volume pumping between chunks.

Studio selects the reply model and effort explicitly (defaults: `gpt-5.6-sol`,
`medium`). The worker validates them against an installed Codex runtime catalog.
See [configuration](../../docs/reference/configuration.md) for discovery and overrides.
