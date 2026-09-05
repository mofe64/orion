# Run Studio Voice for the first time

Before enabling the Orion microphone from an Apple Silicon workstation,
install the Pi listener and Studio processing worker.

Starting Studio does not download the models proactively. Model libraries may
attempt an implicit download when Voice starts, but the first load can exceed
Studio's three-minute worker connection timeout. Prefetching is therefore part
of the required first-time setup.

## Prerequisites

- An Apple Silicon Mac.
- Python 3.12.
- `uv`.
- The configured [Pi listener](../../voice/README.md), its token and a trusted
  local network. Voice audio and the token travel unencrypted.
- The Studio prerequisites from [Run Orion Studio](first-studio-run.md).
- A completed `codex login` if you intend to use the default Codex agent.

The implemented Qwen3-ASR automatic speech-recognition adapter and Chatterbox
text-to-speech adapter use Apple's MLX machine-learning framework. Intel macOS,
Windows, and Linux workstations do not support the complete voice pipeline.

## 1. Install the worker

From the repository root:

```bash
cd orion_studio/voice_worker
uv sync --python 3.12
```

This creates `voice_worker/.venv` and installs the processing worker. It does
not install Rustpotter or download the large ASR and TTS weights.

## 2. Download the configured models

```bash
.venv/bin/orion-voice-models
```

The command downloads and verifies:

- Qwen3-ASR 0.6B.
- Chatterbox Turbo 8-bit.
- Chatterbox's S3 tokenizer.

The weights go to the user's Hugging Face cache, normally
`~/.cache/huggingface/hub`. They are not written into the repository. A
successful command prints the resolved cache directory for each model.

See [model management](../how-to/manage-studio-voice-models.md) for model
overrides and offline preparation.

## 3. Prepare the default agent

```bash
codex login
```

The worker reuses this cached sign-in. It does not require an OpenAI API key.
With this provider, only the confirmed text command is sent to Codex; raw
microphone audio travels only between the Pi and workstation. Do not enable this provider if
that data boundary is inappropriate for the environment.

## 4. Run model-independent checks

```bash
.venv/bin/python -m unittest discover -s tests -v
```

These checks validate processing and the Pi transport using fake models.

## 5. Start Studio

```bash
cd ..
pnpm tauri dev
```

Pair Studio with the Pi gateway once using its token. The desktop app saves
the pairing in the OS credential store and reconnects automatically. Studio
starts its voice worker, which loads models and connects to
`ws://GATEWAY_HOST:7448/`. Open **Voice** to inspect status or mute the microphone.
No workstation microphone permission is needed. Capture defaults on unless
explicitly muted. Keep Studio open for replies. Without Studio, Orion plays an
error cue after capture and returns to listening.
Studio reports Pi capture readiness, wake confirmation, transcription,
response generation and playback.

Say “Hey Orion” followed by a command. Rustpotter on the Pi proposes the wake candidate;
Qwen confirms the phrase and transcribes it before the selected agent sees the
text. The implemented Codex agent may generate speech but cannot move Orion or
invoke other physical capabilities.

## Troubleshooting

- **Worker environment missing:** rerun `uv sync --python 3.12` inside
  `orion_studio/voice_worker`.
- **Startup times out during a download:** stop Voice, run
  `.venv/bin/orion-voice-models`, and retry after all three paths print.
- **Codex authentication fails:** run `codex login` in a terminal and restart
  the voice session.
- **Model or effort is unavailable:** update an installed Codex/ChatGPT app or
  CLI and re-enable Voice. Studio checks each discovered runtime’s model catalog;
  it does not silently substitute another model. Voice → Debug shows the selected
  runtime. An explicit `ORION_STUDIO_CODEX_BIN` override restricts discovery to that
  executable; remove the override to restore automatic discovery.
- **Voice works in neither browser nor Studio:** use `pnpm tauri dev`; the
  UI-only `pnpm dev` server cannot launch the worker.
- **Old TLS configuration:** remove `ORION_PI_VOICE_CA` from the launch
  environment and unset an old `wss://` override in `ORION_PI_VOICE_URL`, or
  replace it with `ws://PI_HOST:7448/`. Reinstall the Pi service template if
  its command still contains `--cert` or `--key`.
- **Pi unavailable:** check `orion-listener.service` and TCP 7448, then retry.
- **Unsupported platform:** Qwen and Chatterbox adapters require Apple Silicon.
