# Orion configuration reference

Orion Studio and the Raspberry Pi deployment tools accept the following
environment variables.

## Orion Studio

`ASR` means automatic speech recognition, and `TTS` means text-to-speech.

| Variable | Default | Purpose |
| --- | --- | --- |
| `ORION_PROJECT_ROOT` | Resolved from the Tauri crate during development | Points a packaged or relocated Studio build at an Orion checkout |
| `ORION_STUDIO_VOICE_PYTHON` | `orion_studio/voice_worker/.venv/bin/python` on macOS | Overrides the Python executable used to start the voice worker |
| `ORION_STUDIO_ASR_MODEL` | `Qwen/Qwen3-ASR-0.6B` | Qwen3-ASR repository ID or compatible local model path |
| `ORION_STUDIO_TTS_MODEL` | `mlx-community/chatterbox-turbo-8bit` | Chatterbox repository ID or compatible local model path |
| `ORION_PI_VOICE_URL` | `ws://GATEWAY_HOST:7448/` | Pi listener endpoint |
| `ORION_STUDIO_AGENT_PROVIDER` | `codex` | Agent adapter name; only `codex` is implemented |
| `ORION_STUDIO_CODEX_BIN` | First installed runtime advertising the selected model and effort | Explicit executable override; when set, automatic fallback is disabled |
| `HF_HOME` | Hugging Face platform default | Relocates the model cache when set for both downloader and Studio |

Choose the reply model and reasoning effort in Studio’s Voice → Reply model section.
The defaults are `gpt-5.6-sol` and `medium`; preferences are saved in `~/.config/orion/voice-settings.json` and
applied with **Save reply settings**, which restarts Studio’s voice worker. Runtime discovery tries installed Codex/ChatGPT app
executables, then the PATH CLI, then the SDK runtime. Each candidate must report
the selected model and effort in its catalog. Debug shows the selected executable.

From the repository root, `./scripts/studio-dev.sh` starts Studio without a
working-directory change or Codex environment override.

Set Studio variables on the same command that starts the Tauri process:

```bash
cd orion_studio
ORION_STUDIO_VOICE_PYTHON=/absolute/path/to/python \
pnpm tauri dev
```

An accepted environment value changes process configuration; it does not prove
that an alternative model or threshold has passed Orion's evaluation.

## Saved pairing

Desktop Studio stores one gateway address/token together in the OS credential
store: macOS Keychain, Windows Credential Manager or Linux Secret Service.
The native service name is `org.orion.studio.pairing` and account is
`paired-orion`. No token is persisted in browser local/session storage.
The UI-only development server keeps a connection in memory for the current
tab only; use the Tauri app for saved pairing.

Studio reconnects on launch and retries network failures with a delay increasing
from 1 to 15 seconds. Each HTTP request has a five-second timeout. Rejected
credentials pause automatic retries and show **Pair Orion again**. **Disconnect**
pauses connection for the current session; **Forget Orion on this computer**
removes the saved credential. Neither action changes Pi torque or character
mode. Closing the Voice panel detaches its UI observer; quitting Studio stops its
voice worker. Pi capture continues unless explicitly muted. If no Studio worker
is connected when capture finishes, Orion plays an error cue and listens again.
Only model and effort are saved in `voice-settings.json`; pairing credentials
remain in the desktop credential store. Microphone mute is a persistent Pi setting.

Linux desktop builds require the Secret Service backend and D-Bus development
libraries (`libdbus-1-dev` on Debian/Ubuntu), plus an unlocked desktop keyring
at runtime. Native persistence was tested on macOS; Windows/Linux integration
still requires platform validation.

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

## Pi listener and character startup

The listener's `--wake-model` defaults to `voice/models/wake/hey_orion_reference.rpw`
and `--threshold` defaults to 0.400. They are Pi settings; Studio has no wake
model settings. Service microphone geometry uses `ORION_MIC_SPACING` (metres)
and `ORION_CHANNEL_SIGN` (-1 or 1) from `~/.config/orion/voice.env`; zero defaults
disable direction estimation pending commissioning. See [Pi voice setup](../../voice/README.md).

`oriond --serve` defaults to `--character-on-start on`. Select `off` explicitly
for torque-off maintenance startup. Studio Stop affects the current daemon
session, not the next restart.

The listener's `--mute-file` defaults to `~/.config/orion/microphone.json`.
Absent a saved preference, capture starts enabled. The authenticated protocol-1
`hello` with `role: "control"` returns microphone status; `microphone.mute` with
a boolean `muted` changes and persists it without claiming processing ownership.
Starting `oriond.service` pulls in the listener; `PartOf=oriond.service` propagates
runtime stop/restart to the listener.
