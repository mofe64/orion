# Orion Studio voice worker

The worker keeps Orion's wake, transcription, agent, and speech models behind
one authenticated loopback WebSocket. Qwen3-ASR provides automatic speech
recognition (ASR), Chatterbox provides text-to-speech (TTS), and
`AgentProvider` is the small adapter contract between them:

```text
Rustpotter -> Qwen3-ASR -> AgentProvider -> Chatterbox Turbo -> Orion ReSpeaker
```

Rustpotter proposes a wake candidate. Qwen confirms that the transcript starts
with “Hey Orion” and returns the remaining command. The configured agent sees
only that confirmed text, and Chatterbox converts its reply to digital audio.

## Setup on Apple Silicon

The Qwen and Chatterbox adapters use Apple's MLX machine-learning framework and
require an Apple Silicon Mac with Python 3.10–3.13, `uv`, and a Rust toolchain:

```bash
cd orion_studio/voice_worker
uv sync --python 3.12
.venv/bin/orion-voice-models
```

Run both commands on each development device before enabling Voice.
Starting Studio does not proactively fetch the models. If they are absent when
Voice starts, the libraries may download them during worker startup and exceed
Studio's three-minute connection timeout. The explicit downloader is the
supported, observable first-run path.

The download command places the weights in the user's Hugging Face cache. The
commissioned models use about 1.8 GB for Qwen, 675 MB for Chatterbox Turbo
8-bit, and 472 MB for its tokenizer. The weights are not included in Git or the
Studio bundle. See [model management](../../docs/how-to/manage-studio-voice-models.md)
for cache and offline preparation. Select other compatible models for one
Studio run with:

```bash
cd ..
ORION_STUDIO_ASR_MODEL=/absolute/path/to/qwen3-asr-model \
ORION_STUDIO_TTS_MODEL=/absolute/path/to/chatterbox-model \
pnpm tauri dev
```

The default agent provider uses the cached sign-in from `codex login`; it does
not need an OpenAI API key. `ORION_STUDIO_AGENT_MODEL` optionally selects a
Codex model. The worker depends only on the small `AgentProvider` contract, so
an OpenAI Platform or local-LLM adapter can be added without changing wake,
ASR, TTS, or playback.

Rustpotter uses `models/hey_orion_reference.rpw` at threshold `0.400`. Both can
be overridden for an evaluation build:

```bash
ORION_STUDIO_WAKE_MODEL=/absolute/path/to/reference.rpw \
ORION_STUDIO_WAKE_THRESHOLD=0.385 \
pnpm tauri dev
```

## Runtime flow

Studio sends continuous 16 kHz mono signed 16-bit pulse-code modulation (PCM16)
frames after the user enables Voice.
The worker keeps a three-second in-memory pre-roll and follows this sequence:

```text
Rustpotter candidate
    -> endpoint active utterance
    -> Qwen transcript
    -> reject if it does not start with "Hey Orion"
    -> otherwise remove the wake phrase
    -> configured agent provider
    -> Chatterbox PCM16
    -> Studio validates and uploads PCM16 WAV to Orion
    -> Pi ReSpeaker + speaking motion + warm red-green-blue-white (RGBW) light
    -> terminal playback acknowledgement
```

If the first transcript contains only “Hey Orion”, the worker opens a second
endpointed capture for the command. Audio remains in memory and the socket
accepts only one Studio connection.

The version-4 protocol sends JSON state events and binary PCM speech on the
same socket. Studio acknowledges playback only after the Pi reports a terminal
speech run. The worker ignores microphone frames until that acknowledgement,
preventing Orion's own ReSpeaker response from reactivating the wake path.

## Validation

Model-independent tests:

```bash
.venv/bin/python -m unittest discover -s tests -v
```

Native detector compilation:

```bash
PYO3_PYTHON="$PWD/.venv/bin/python" \
  cargo check --manifest-path rustpotter_native/Cargo.toml
```

The recordings, evaluator, and generated reports are outside Orion core in
`../../../voice-model-lab/wakeword` at the shared project-workspace level.
