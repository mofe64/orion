# Orion configuration

The voice owner saves preferences on the Pi for onboard installations.
Studio stores its pairing credentials in the desktop OS credential store. The Pi keeps its own microphone preference, calibration, and
runtime options. Use the [quickstart](quickstart.md) for launch and update commands.

## Orion Studio

Settings saves the agent model and effort, ASR and TTS model IDs, optional model
folders, and download cache to `~/.config/orion/voice-settings.json`. Defaults are
`gpt-5.6-sol` with `medium` effort, `Qwen/Qwen3-ASR-0.6B`, and
`pocket-fp32`. Local model folders take precedence over
model IDs and must contain weights compatible with the selected engine.

Changing only `ttsVoice` applies to the next response without a restart.
Other model settings restart the coordinator and speech workers. An idle restart
preserves the agent conversation when its model, effort, and executable stay the
same. Restarting the whole service starts a fresh conversation. Personality and
memory remain in `~/.local/share/orion/SOUL.md` and `MEMORY.md`.

The agent discovers installed Codex/ChatGPT app runtimes, then the CLI on `PATH`.
A runtime must advertise the selected model and effort and have a working login.
An explicit executable override disables fallback. Studio Debug reports the
selected runtime and loaded speech models.

| Variable | Meaning |
| --- | --- |
| `ORION_PROJECT_ROOT` | Orion source root; defaults to the shared crate's parent during development and the active Pi release |
| `ORION_STUDIO_VOICE_PYTHON` | Development override for `speech/.venv/bin/python`; managed releases use their own environment |
| `ORION_STUDIO_CODEX_BIN` | Absolute path to the Codex executable |
| `ORION_PI_VOICE_URL` | Overrides the listener endpoint derived as `ws://GATEWAY_HOST:7448/` |
| `ORION_STUDIO_ASR_MODEL`, `ORION_STUDIO_TTS_MODEL` | Initial model defaults when no settings file exists |
| `ORION_SOUL_PATH`, `ORION_MEMORY_PATH` | Override the agent's personality and memory files |
| `HF_HOME` | Initial download cache when no settings file exists; also used by model download tools |

Paths in Settings accept absolute paths or `~/`. Changing the cache affects
future downloads and does not move existing weights. The speech worker applies
a saved cache to its `HF_HOME` and `HF_HUB_CACHE` environment.

Set service overrides in the Pi’s `~/.config/orion/voice-stack.env`, then restart
`orion-voice-stack`. Mac environment variables do not configure Pi inference.

## Pi voice profile

The Pi service reads `~/.config/orion/voice-stack.env`. Initial models are
Qwen3-ASR-0.6B GGUF, `pocket-fp32`, and the `alba` preset. Settings offers
`pocket-int8` and the Anna, Azelma, Cosette, Eve, Fantine, Jane, and Vera presets.
Voice, agent, memory, and personality changes are saved on the Pi through the gateway.

| Variable | Meaning |
| --- | --- |
| `ORION_ONBOARD=1` | Use the local Pi listener, gateway, and token file |
| `ORION_SPEECH_BACKEND=pi` | Select Qwen GGUF and Pocket CPU adapters |
| `ORION_ASR_MODEL_DIR` | Folder containing Qwen `model.gguf` and `mmproj.gguf` |
| `ORION_LLAMA_SERVER` | Pinned native Qwen server executable |
| `ORION_ASR_CONTEXT` | Qwen vocabulary context; defaults to `Orion` |
| `ORION_ASR_THREADS`, `ORION_TTS_THREADS` | CPU inference threads, 1–4; managed default 3 |
| `ORION_VAD_MODEL` | Pinned Silero ONNX model supplied to the listener |
| `ORION_CAPTURE_GAIN_DB` | Capture PGA gain, 0–50 dB; managed default 25 |
| `HF_HUB_OFFLINE=1` | Load prepared speech assets from the local cache |

The agent uses the Pi’s Codex sign-in and still requires internet access. Speech
model downloads are inventoried under `~/.local/share/orion/voice-stack`.

## Pi service control

Systemd runs `orion-service` through `orion-voice-stack.service`. It starts at boot
and restarts after failure. Logs are available with `journalctl -u orion-voice-stack`.
There is no Mac background-service installation or launchd job.

The private loopback discovery file and owner lock remain under
`~/.local/share/orion/studio-service/` for compatibility with installed Pi gateways.
The directory name is historical; it contains no desktop service installation.
`ORION_STUDIO_SERVICE_HOME` overrides this path for both service and gateway.
Never unlink `owner.lock` while a service can own it.

Model files, environments, and release sources live under
`~/.local/share/orion/voice-stack/`. Keep its active release and the managed Python
interpreters used by its environments. Download archives and experiment recordings
are not runtime dependencies.

## Saved pairing

Desktop Studio saves one gateway address/token in the native
credential entry `org.orion.studio.pairing`, account `paired-orion`. macOS uses
Keychain; desktop Windows and Linux use Credential Manager and Secret Service.
Linux needs D-Bus development libraries and an unlocked keyring. Native credential
persistence has been validated on macOS; Windows and Linux require platform tests.

The UI reconnects to the gateway with delays increasing from one to fifteen
seconds and a five-second HTTP timeout. Rejected credentials prompt pairing
again. **Disconnect** pauses the UI connection. **Forget Orion on this computer**
deletes the desktop credential. The Pi continues processing voice. Neither changes torque or
character mode. Browser-only development retains pairing in memory for that tab.

Closing Studio leaves Pi processing active. The Pi service retries coordinator
startup every five seconds. Microphone mute is a separate, persistent Pi setting.

## Raspberry Pi deployment

Command-line flags override the deployment environment:

| Variable | Default | Flag |
| --- | --- | --- |
| `ORION_PI_HOST` | `mofe@orion.local` | `--host USER@HOST` |
| `ORION_PI_ROOT` | `/home/mofe/dev/orion` | `--root PATH` |
| `ORION_PI_BRANCH` | `main` | `--branch BRANCH` |

The script validates these values before SSH and requires a trusted host key.
Updates change release-owned executable paths, `ORION_STUDIO_VOICE_PYTHON`, and
`ORION_RELEASE_REVISION` (also `ORION_PROJECT_ROOT` if explicitly configured).
Existing environment values, systemd override arguments, calibration and saved
preferences survive. Defaults are added only when absent. Runtime and gateway
retain the existing motion/user-asset root. See the
[update and rollback behavior](quickstart.md#deploy-to-the-pi).
See [Pi deployment](quickstart.md#deploy-to-the-pi).

## Pi runtime and listener

`oriond` accepts command-line options for its backend, socket, serial port,
calibration, scene catalog, Python executable, and start pose. Inspect them with
`runtime/target/release/oriond --help` and use the
[runtime commands](../runtime/README.md) for hardware and MuJoCo startup. Audio
card, RGBW geometry, GPIO, and device helper paths are compiled into the runtime.

`--character-on-start` defaults to `on`; use `off` for torque-off maintenance
startup. `--rest-after-seconds` defaults to `1800` (30 minutes) and accepts a finite positive
number. Only accepted ASR wake confirmations reset inactivity. See
[automatic rest](system-architecture.md#automatic-rest-and-waking) for motion,
light, and torque behavior.

The listener defaults to `voice/models/wake/hey_orion_reference.rpw` and a `0.400`
wake threshold. `ORION_MIC_SPACING` in metres and `ORION_CHANNEL_SIGN` in
`~/.config/orion/voice.env` configure direction estimation; zero defaults disable
it pending commissioning. Microphone mute persists in
`~/.config/orion/microphone.json`, with capture enabled when no preference exists.
Starting `oriond.service` pulls in the listener, and runtime stop/restart propagates
to it. See [Pi voice setup](../voice/README.md) for commissioning and controls.

The managed Pi listener uses Rustpotter threshold `0.35` with 25 dB capture gain.
Qwen must still confirm the wake phrase before the agent runs. This setting was
checked with onboard speaker recordings; distant human speech and household
noise require broader validation. Standalone listener defaults remain unchanged.
