# Orion configuration reference

Orion Studio and the Raspberry Pi deployment tools accept the following
environment variables.

## Orion Studio

| Variable | Default | Purpose |
| --- | --- | --- |
| `ORION_PROJECT_ROOT` | Resolved from the Tauri crate during development | Points a packaged or relocated Studio build at an Orion checkout |
| `ORION_STUDIO_VOICE_PYTHON` | `orion_studio/voice_worker/.venv/bin/python` on Unix; `.venv/Scripts/python.exe` on Windows | Overrides the Python executable used to start the voice worker |
| `ORION_STUDIO_ASR_MODEL` | `Qwen/Qwen3-ASR-0.6B` | Qwen3-ASR repository ID or compatible local model path |
| `ORION_STUDIO_TTS_MODEL` | `mlx-community/chatterbox-turbo-8bit` | Chatterbox repository ID or compatible local model path |
| `ORION_STUDIO_WAKE_MODEL` | `voice_worker/models/hey_orion_reference.rpw` | Rustpotter reference file |
| `ORION_STUDIO_WAKE_THRESHOLD` | `0.400` | Rustpotter candidate threshold in the range `(0, 1]` |
| `ORION_STUDIO_AGENT_PROVIDER` | `codex` | Agent adapter name; only `codex` is implemented |
| `ORION_STUDIO_AGENT_MODEL` | Codex configured default | Optional Codex model override |
| `HF_HOME` | Hugging Face platform default | Relocates the model cache when set for both downloader and Studio |

Set Studio variables on the same command that starts the Tauri process:

```bash
cd orion_studio
ORION_STUDIO_WAKE_THRESHOLD=0.385 \
ORION_STUDIO_AGENT_MODEL=MODEL_NAME \
pnpm tauri dev
```

An accepted environment value changes process configuration; it does not prove
that an alternative model or threshold has passed Orion's evaluation.

## Raspberry Pi deployment

The deployment script accepts command-line flags or these environment
variables:

| Variable | Default | Equivalent flag |
| --- | --- | --- |
| `ORION_PI_HOST` | `mofe@orion.local` | `--host USER@HOST` |
| `ORION_PI_ROOT` | `/home/mofe/dev/orion` | `--root PATH` |
| `ORION_PI_BRANCH` | `main` | `--branch BRANCH` |

Explicit flags replace environment values. The script validates the target,
path, and branch before opening SSH and never disables host-key checking.

```bash
scripts/deploy_pi.sh \
  --host USER@HOST \
  --root /absolute/path/on/pi \
  --branch main
```

## Runtime command options

`oriond` uses command-line options rather than environment variables for its
backend, socket, serial port, calibration, scene catalog, Python executable,
and start pose. Run:

```bash
runtime/target/release/oriond --help
```

See [runtime commands](../../runtime/README.md) for the normal MuJoCo and
hardware sequences. The ReSpeaker card, RGBW dimensions, GPIO, and executable
paths are compiled into the runtime and cannot be overridden through the
environment.
