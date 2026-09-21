# Orion configuration

Voice settings, microphone mute, calibration, personality and memories are saved
on the Pi. Studio stores its pairing credentials and UI preferences on the
desktop. Use the [quickstart](quickstart.md) for installation, updates and service
recovery.

## Saved files

Paths below are relative to the Pi user's home unless stated otherwise.

| Path | Contents |
| --- | --- |
| `.config/orion/voice-settings.json` | Agent model and effort, speech models, optional model/cache paths and voice preset |
| `.config/orion/voice-stack.env` | Environment for the onboard voice and agent service |
| `.config/orion/voice.env` | Listener overrides, including microphone spacing, channel orientation and capture gain |
| `.config/orion/routines.json` | Idle/lamp mode, pending timers and one-time alarms, recent alert results and ringing deadline |
| `.config/orion/routines.sounds.json` | Alarm and timer sound preferences, plus the sound selected for the most recent ringing window |
| `.config/orion/microphone.json` | Persistent microphone mute preference |
| `.config/orion/servo_calibration.json` | Joint zeros, directions and calibrated ranges |
| `.config/orion/studio-token` | Gateway and listener authentication token |
| `.local/share/orion/SOUL.md` | Saved personality choices and generated instructions |
| `.local/share/orion/MEMORY.md` | Explicitly saved memories |
| `.local/share/orion/voice-stack/` | Releases, shared models, native tools and deployment inventories |

The runtime and gateway read assets from the existing catalog root, normally
`/home/mofe/dev/orion`. This is separate from the code release selected by systemd.
See [assets and installed releases](system-architecture.md#assets-and-installed-releases).

## Voice settings

Studio Settings saves preferences through the Pi gateway. The defaults are
`gpt-5.6-sol` with `medium` effort, `Qwen/Qwen3-ASR-0.6B`, `pocket-fp32` and
`alba`. The Pi adapter loads Qwen GGUF files from the configured local folder.
Pocket accepts the FP32 or INT8 model choice and a named preset.

Presets are Alba, Anna, Azelma, Cosette, Eve, Fantine, Jane and Vera. Studio
**Settings → Voice and sounds → Default Pocket voice** saves `ttsVoice` automatically
on the Pi and applies it to the next response. Studio requires mute before other model
changes, which restart the coordinator and speech workers. An idle restart can
preserve the agent conversation when its model, effort and executable still
match. Restarting the whole service starts a fresh conversation.

Saved model folders take precedence over model IDs. Paths accept an absolute
folder or `~/`; they must contain files accepted by the selected engine. The Pi
UI displays model paths as read-only. A saved cache path is passed to the worker
as `HF_HOME` and `HF_HUB_CACHE`; changing it does not move existing weights.

When no settings file exists, `ORION_STUDIO_ASR_MODEL`,
`ORION_STUDIO_TTS_MODEL` and `HF_HOME` supply initial defaults. Existing saved
choices take precedence. `ORION_ASR_MODEL_DIR` supplies the onboard ASR folder
default. This distinction matters when changing service environment values on
an installation that already has saved settings.

## Pi voice profile

Set voice-host overrides in `~/.config/orion/voice-stack.env`, then restart
`orion-voice-stack`. Set listener overrides in `~/.config/orion/voice.env`, then
restart `orion-listener`. Systemd units and their `.service.d/*.conf` files can
also supply environment values or command arguments. Inspect the effective
configuration with `systemctl cat SERVICE` before editing it.

| Variable | Meaning |
| --- | --- |
| `ORION_ONBOARD=1` | Use the Pi's local listener, gateway and token file |
| `ORION_SPEECH_BACKEND=pi` | Select Qwen GGUF and Pocket CPU adapters |
| `ORION_PROJECT_ROOT` | Source directory for the voice service; managed installs select the active release |
| `ORION_STUDIO_VOICE_PYTHON` | Speech Python executable; managed installs select the release's own environment |
| `ORION_STUDIO_CODEX_BIN` | Explicit Codex executable; the installer selects its pinned native package |
| `ORION_ASR_MODEL_DIR` | Folder containing Qwen `model.gguf` and `mmproj.gguf` |
| `ORION_LLAMA_SERVER` | Native Qwen server executable |
| `ORION_ASR_CONTEXT` | Vocabulary hint supplied to Qwen; default `Orion` |
| `ORION_ASR_THREADS`, `ORION_TTS_THREADS` | CPU inference threads, 1–4; managed default 3 |
| `HF_HOME` | Speech model cache and initial saved-cache default |
| `HF_HUB_OFFLINE=1` | Use prepared speech assets from the local cache |
| `ORION_RELEASE_REVISION` | Release identifier reported by the voice host |
| `ORION_PI_VOICE_URL` | Override the listener WebSocket URL; onboard default `ws://127.0.0.1:7448/` |
| `ORION_SOUL_PATH`, `ORION_MEMORY_PATH` | Override the agent's personality and memory files |

An explicit Codex executable disables discovery fallback. Without one, the agent
checks supported installed app runtimes and then the CLI on `PATH`. It verifies
login and the advertised model/effort combinations. Studio uses that advertised
catalog for its choices. The Pi login procedure is in the
[quickstart](quickstart.md#pi-local-voice-and-agent).

## Pi service control

Systemd runs the voice host as `orion-voice-stack`, the listener as
`orion-listener`, the gateway as `orion-studio-gateway`, and the hardware runtime
as `oriond`. The voice host retries a stopped coordinator every five seconds.
The listener continues capture according to its saved mute state when Studio
closes.

The private service discovery file and owner lock live under
`~/.local/share/orion/studio-service/`. That directory name is retained for gateway
compatibility. `ORION_STUDIO_SERVICE_HOME` overrides it for both service and
gateway. Keep `owner.lock` in place while a service can own it.

Models and native tools are shared under `~/.local/share/orion/voice-stack/`.
Each release has its own Python environments and runtime binaries. Those
environments also depend on the managed Python interpreters used to create them.
The [retention guidance](quickstart.md#retained-files) explains what a working
release and rollback need.

## Saved pairing

Desktop Studio saves the gateway address and token in the credential entry
`org.orion.studio.pairing`, account `paired-orion`. macOS uses Keychain; Windows
uses Credential Manager; Linux uses Secret Service and needs an unlocked keyring.
The native credential test is optional and separate from ordinary integration
tests. Browser development keeps pairing in memory for the current tab.

The UI reconnects with delays from one to fifteen seconds and a five-second HTTP
timeout. Rejected credentials request pairing again. **Disconnect** pauses retries
for that session. **Forget Orion on this computer** removes the desktop credential.
Voice processing and saved microphone mute continue on the Pi.

## Raspberry Pi deployment

Command-line flags override the deployment environment:

| Variable | Default | Flag |
| --- | --- | --- |
| `ORION_PI_HOST` | `mofe@orion.local` | `--host USER@HOST` |
| `ORION_PI_ROOT` | `/home/mofe/dev/orion` | `--root PATH` |
| `ORION_PI_BRANCH` | `main` | `--branch BRANCH` |

The script validates these values before SSH and requires a trusted host key.
It builds from the selected remote commit. The Pi checkout, including its local
edits, remains the catalog and Git source for preparing releases.

Activation changes executable paths in the base units and overrides, the speech
Python path, the release revision, and the voice service's project root. Existing
environment values and command arguments are preserved; missing defaults are
added. Saved preferences, calibration and token files stay outside that write set.
Rollback restores the immediately previous service configuration and running
state. See [Pi deployment](quickstart.md#deploy-to-the-pi).

## Pi runtime and listener

`oriond` accepts options for its backend, socket, serial port, calibration, asset
paths, Python executable and start pose. `--character-on-start` defaults to `on`.
Use `off` for maintenance startup with torque disabled. `--rest-after-seconds`
defaults to `1800`, or 30 minutes. Accepted wake confirmations reset inactivity;
active conversations and foreground work can defer rest. Lamp mode disables
automatic rest while preserving idle animations. See
[automatic rest](system-architecture.md#automatic-rest-and-waking).

`--routines-file PATH` overrides the saved mode and alert file. Hardware defaults
to `~/.config/orion/routines.json`; simulation keeps this state in memory unless
a file is supplied. Sound settings use the same path with its extension replaced
by `.sounds.json`. The installer preserves both files. Writes use atomic
replacement without forcing a storage flush in the motion loop; a sudden power
loss can lose a recent change. Clock alarms use
the Pi’s local time and explicit UTC offsets; verify its timezone and clock before
relying on a clock alarm.

The managed listener uses the Rustpotter reference
`voice/models/wake/hey_orion_reference.rpw`, wake threshold `0.45` and 25 dB capture
gain. The standalone CLI defaults to threshold `0.400`; the capture-routing script
defaults to 50 dB when no override is supplied. `ORION_CAPTURE_GAIN_DB` accepts
0–50 dB. These separate defaults make the effective service configuration the
relevant setting for a running Pi.

`ORION_VAD_MODEL` selects the listener's Silero ONNX file. `ORION_MIC_SPACING`, in
metres, and `ORION_CHANNEL_SIGN`, as `1` or `-1`, configure direction estimation.
Their zero defaults disable direction estimates until the microphone geometry
is checked. Restart the listener after changing them. Starting `oriond` pulls in
the listener, and runtime stop/restart propagates to it.

Qwen still confirms the wake phrase after Rustpotter detection. Household noise,
distant speech and speaker pickup require physical validation. See
[capture and session behavior](voice-architecture.md#capture-ownership-and-session-lifecycle)
and [audio setup](../hardware/audio/README.md).
