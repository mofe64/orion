# Orion Studio processing worker

The worker receives Pi-triggered utterances, confirms the wake phrase with
Qwen3-ASR, extracts the command, invokes the configured agent and synthesizes
speech with Chatterbox. It never captures a workstation microphone or loads
Rustpotter. The Pi owns the sole wake detector.

## Setup on Apple Silicon

The Qwen and Chatterbox adapters require Apple Silicon and Python 3.10–3.13.
From this directory:

```bash
uv sync --python 3.12
.venv/bin/orion-voice-models
.venv/bin/python -m unittest discover -s tests -v
```

The model downloader prepares the Hugging Face cache. Studio startup does not
proactively download weights. See [model management](../../docs/how-to/manage-studio-voice-models.md).

Prepare the [Pi listener](../../voice/README.md), then launch Studio. No
certificate setup is required. The native launcher passes the listener URL
and authentication token to the worker. The token is
passed in the child environment, not in the URL or logged command arguments.
`ORION_PI_VOICE_URL` overrides the default `ws://GATEWAY_HOST:7448/`.
The token and audio travel unencrypted on the local network.

## Processing contract

Local protocol version 5 accepts UI control and playback acknowledgements only.
The authenticated loopback worker connects to Pi listener protocol version 1.
Qwen rejects transcripts that do not begin with “Hey Orion”; an empty confirmed
wake requests a follow-up utterance buffered by the Pi. Only extracted command
text reaches the configured agent. The Codex provider produces spoken replies
and cannot invoke movement or raw device operations.

The worker sends Chatterbox PCM16 to Studio, which uploads validated mono
24 kHz WAV to Orion through the existing gateway. It acknowledges playback
only after a terminal Pi speech run. Failures discard the active session;
connection failure stops Voice and requires an explicit retry. Microphone
frames sent by Studio are protocol errors.

Model-independent tests include real loopback WebSocket integration with fake
models; they do not establish physical microphone quality or inference latency.
See [voice architecture](../../docs/explanation/voice-architecture.md) for data
flow, deadlines, privacy and the separate gateway transport boundary.
