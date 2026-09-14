# Orion configuration

Studio saves voice preferences on the Mac and pairing credentials in the OS
credential store. The Pi keeps its own microphone preference, calibration, and
runtime options. Use the [quickstart](quickstart.md) for launch and update commands.

## Orion Studio

Settings saves the agent model and effort, ASR and TTS model IDs, optional model
folders, and download cache to `~/.config/orion/voice-settings.json`. Defaults are
`gpt-5.6-sol` with `medium` effort, `Qwen/Qwen3-ASR-0.6B`, and
`mlx-community/chatterbox-turbo-8bit`. Local model folders take precedence over
model IDs and must contain weights compatible with the selected engine.

Saving settings restarts the coordinator and speech worker. An idle restart
preserves the agent conversation when its model, effort, and executable stay the
same. Restarting the whole service starts a fresh conversation. Personality and
memory remain in `~/.local/share/orion/SOUL.md` and `MEMORY.md`.

The agent discovers installed Codex/ChatGPT app runtimes, then the CLI on `PATH`.
A runtime must advertise the selected model and effort and have a working login.
An explicit executable override disables fallback. Studio Debug reports the
selected runtime and loaded speech models.

| Variable | Meaning |
| --- | --- |
| `ORION_PROJECT_ROOT` | Orion source root; defaults to the shared crate's parent during development and the active release in headless mode |
| `ORION_STUDIO_VOICE_PYTHON` | Development override for `speech/.venv/bin/python`; managed releases use their own environment |
| `ORION_STUDIO_CODEX_BIN` | Absolute path to the Codex executable |
| `ORION_PI_VOICE_URL` | Overrides the listener endpoint derived as `ws://GATEWAY_HOST:7448/` |
| `ORION_STUDIO_ASR_MODEL`, `ORION_STUDIO_TTS_MODEL` | Initial model defaults when no settings file exists |
| `ORION_SOUL_PATH`, `ORION_MEMORY_PATH` | Override the agent's personality and memory files |
| `HF_HOME` | Initial download cache when no settings file exists; also used by model download tools |

Paths in Settings accept absolute paths or `~/`. Changing the cache affects
future downloads and does not move existing weights. The speech worker applies
a saved cache to its `HF_HOME` and `HF_HUB_CACHE` environment.

Set development overrides when launching Studio:

```bash
ORION_STUDIO_CODEX_BIN=/absolute/path/to/codex scripts/studio-dev.sh
```

## Headless service

The macOS LaunchAgent lives at
`~/Library/LaunchAgents/org.orion.studio.headless.plist`. It runs as the logged-in
user, starts at login, and restarts after a crash. It needs an awake Mac, accessible
login credentials, and connectivity to the Pi.

Service files live under `~/.local/share/orion/studio-service/`:

| Path | Contents |
| --- | --- |
| `releases/` | Source snapshots, compiled binaries, and a Python environment per release |
| `current` | Symlink selecting the active release |
| `logs/stdout.log`, `logs/stderr.log` | launchd process output |
| `installed` | Directs the UI to the managed owner even while it is stopped |
| `owner.lock` | OS-held lock preventing simultaneous desktop and headless owners |
| `connection.json` | Private loopback address and temporary service token |
| `update.lock`, `build/` | Update serialization and reusable Cargo build output |

The installer captures `PATH` and the supported agent, listener, model, and cache
overrides from its environment into the LaunchAgent. Shell edits do not change a
running job. Apply environment changes through the update command; pairing tokens
remain in the credential store. The release supplies its own source root and
Python environment.

`ORION_STUDIO_SERVICE_HOME` overrides the service directory. Use the same value
for the UI and management scripts. `ORION_STUDIO_LAUNCHD_LABEL` and
`ORION_STUDIO_LAUNCH_AGENTS` select an alternate label and registration directory
for isolated testing. Automatic login discovery requires the normal
`~/Library/LaunchAgents` directory.

## Saved pairing

Desktop and headless Studio share one gateway address/token in the native
credential entry `org.orion.studio.pairing`, account `paired-orion`. macOS uses
Keychain; desktop Windows and Linux use Credential Manager and Secret Service.
Linux needs D-Bus development libraries and an unlocked keyring. Native credential
persistence has been validated on macOS; Windows and Linux require platform tests.

The UI reconnects to the gateway with delays increasing from one to fifteen
seconds and a five-second HTTP timeout. Rejected credentials prompt pairing
again. **Disconnect** pauses the UI connection. **Forget Orion on this computer**
deletes the credential and stops its voice processing. Neither changes torque or
character mode. Browser-only development retains pairing in memory for that tab.

With headless installed, closing Studio leaves processing active. Without it,
quitting Studio stops its embedded coordinator and speech worker. The headless
owner retries coordinator startup every five seconds when it is stopped; Pi
network reconnection belongs to the coordinator. Microphone mute is a separate,
persistent Pi setting.

## Raspberry Pi deployment

Command-line flags override the deployment environment:

| Variable | Default | Flag |
| --- | --- | --- |
| `ORION_PI_HOST` | `mofe@orion.local` | `--host USER@HOST` |
| `ORION_PI_ROOT` | `/home/mofe/dev/orion` | `--root PATH` |
| `ORION_PI_BRANCH` | `main` | `--branch BRANCH` |

The script validates these values before SSH and requires a trusted host key.
See [Pi deployment](quickstart.md#deploy-to-the-pi).

## Pi runtime and listener

`oriond` accepts command-line options for its backend, socket, serial port,
calibration, scene catalog, Python executable, and start pose. Inspect them with
`runtime/target/release/oriond --help` and use the
[runtime commands](../runtime/README.md) for hardware and MuJoCo startup. Audio
card, RGBW geometry, GPIO, and device helper paths are compiled into the runtime.

`--character-on-start` defaults to `on`; use `off` for torque-off maintenance
startup. `--rest-after-seconds` defaults to `600` and accepts a finite positive
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
