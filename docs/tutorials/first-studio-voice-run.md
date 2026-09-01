# Run Studio Voice for the first time

On a new Apple Silicon development device, install the persistent worker and
download its models before enabling Studio microphone capture.

Starting Studio does not download the models proactively. Model libraries may
attempt an implicit download when Voice starts, but the first load can exceed
Studio's three-minute worker connection timeout. Prefetching is therefore part
of the required first-time setup.

## Prerequisites

- An Apple Silicon Mac.
- Python 3.10–3.13; Python 3.12 is the commissioned choice.
- `uv`.
- A Rust toolchain for the native Rustpotter extension.
- The Studio prerequisites from [Run Orion Studio](first-studio-run.md).
- A completed `codex login` if you intend to use the default Codex agent.

Qwen3-ASR and Chatterbox currently use MLX. The complete voice pipeline is not
supported on Intel macOS, Windows, or Linux workstations yet.

## 1. Install the worker

From the repository root:

```bash
cd orion_studio/voice_worker
uv sync --python 3.12
```

This creates `voice_worker/.venv`, installs the Python worker, and builds the
editable native Rustpotter adapter. It does not download the large ASR and TTS
weights.

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
microphone audio remains on the workstation. Do not enable this provider if
that data boundary is inappropriate for the environment.

## 4. Run model-independent checks

```bash
.venv/bin/python -m unittest discover -s tests -v
PYO3_PYTHON="$PWD/.venv/bin/python" \
  cargo check --manifest-path rustpotter_native/Cargo.toml
```

These checks validate the protocol and detector build without loading the
large models.

## 5. Start Studio

```bash
cd ..
pnpm tauri dev
```

Open **Voice** and select **Enable microphone**. Approve the operating-system
permission when prompted. Studio shows worker startup, input device, native
sample rate, level, wake state, transcript, agent response, and playback.

Say “Hey Orion” followed by a command. Rustpotter proposes the wake candidate;
Qwen confirms the phrase and transcribes it before the selected agent sees the
text. The current agent may generate speech but cannot move Orion or invoke
other physical capabilities.

## Troubleshooting

- **Worker environment missing:** rerun `uv sync --python 3.12` inside
  `orion_studio/voice_worker`.
- **Startup times out during a download:** stop Voice, run
  `.venv/bin/orion-voice-models`, and retry after all three paths print.
- **Codex authentication fails:** run `codex login` in a terminal and restart
  the voice session.
- **Voice works in neither browser nor Studio:** use `pnpm tauri dev`; the
  UI-only `pnpm dev` server cannot launch the worker.
- **Unsupported platform:** check the [platform matrix](../reference/platform-support.md).
