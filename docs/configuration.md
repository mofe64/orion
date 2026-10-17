# Orion configuration

Orion stores settings under the Pi login user's home directory. Installed releases use a separate shared data directory. Updates preserve user settings and user-authored assets.

## Saved files

Paths in this table are relative to the Pi user's home directory.

| Path | Content |
| --- | --- |
| `.config/orion/voice-settings.json` | Saved agent and speech choices |
| `.config/orion/voice-stack.env` | Service executable, model and worker settings |
| `.config/orion/voice.env` | Microphone and wake settings |
| `.config/orion/routines.json` | Mode, timers, alarms and timed reminders |
| `.config/orion/routines.sounds.json` | Alarm and timer sounds |
| `.config/orion/microphone.json` | Microphone mute |
| `.config/orion/camera.json` | Camera privacy |
| `.config/orion/tracking.json` | Tracking mode |
| `.config/orion/vision.json` | Vision accelerator selection |
| `.config/orion/behaviours.json` | Optional character behaviors |
| `.config/orion/servo_calibration.json` | V1 servo calibration |
| `.config/orion/studio-token` | Gateway and listener access token |
| `.local/share/orion/SOUL.md` | Personality choices |
| `.local/share/orion/mind.json` | Runtime energy, mood, activity and attention |
| `.local/share/orion/MEMORY.md` | Saved memories |
| `.local/share/orion/MEMORY.automatic.json` | Automatic-memory duplicate protection |
| `.local/share/orion/weather.json` | Weather location and temperature unit |
| `.local/share/orion/lists.json` | Shopping and to-do lists |
| `.local/share/orion/reminders.json` | Next-conversation reminders |
| `.local/share/orion/voice-stack/` | Releases, transactions, models and catalogue |

Memory-related files follow a custom `ORION_MEMORY_PATH` location when configured. V2 uses the tracked `hardware/profiles/v2-joints.json` file instead of user calibration.

The default asset project is `~/.local/share/orion/voice-stack/catalog/`. User assets occupy `motion/user/poses/<version>/`, `motion/motions/<version>/user/` and `scenes/<version>/user/` within that project. `ORION_INSTALL_ROOT` changes the installer data root.

## Character behaviors

`oriond` writes `mind.json` through atomic replacement with mode `0600`. It saves at most once per ten seconds and at shutdown.
Saved timestamps preserve activity and mood decay across restarts. A missing or invalid file selects default state.
The agent omits state older than ten minutes. Studio Debug shows the state through `/api/v2/status`.

`behaviours.json` accepts the boolean keys in this table. Missing keys use the listed defaults.

| Key | Default | Function |
| --- | --- | --- |
| `object_glances` | false | Look briefly at mapped desk objects |
| `mutual_gaze` | false | Respond to a sustained look |
| `arrival_notice` | false | Welcome a person after prolonged absence |
| `new_object_peek` | false | Look at a new object or pet |
| `presence_defers_rest` | false | Delay automatic rest while people or hands remain visible |
| `mind_idle_rhythm` | true | Adjust idle rhythm and background light with energy |
| `listen_lean_in` | true | Hold a small lean while listening |
| `thinking_beats` | true | Add alternating gestures during longer thoughts |
| `wake_sleep_acts` | true | Stretch after confirmed wake and settle before rest |
| `posture_drift` | false | Change the held posture during quiet idle periods |
| `act_sounds` | true | Play sounds at wake, sleep and yawn markers |

Studio provides these switches under **Character behaviours**. The runtime reads act settings independently of camera observations.
The new act clips require the V2 catalogue. See [behavior ownership](motion-architecture.md#autonomous-behaviors).

## Camera privacy and tracking profile

`camera.json` stores `camera_enabled`. A missing file permits capture. An invalid or unreadable file disables capture.

`tracking.json` stores `tracking_mode`: `off`, `idle`, `faces` or `hands`. A missing file selects Faces. Invalid settings select Off. Camera privacy disables capture in every mode.

`hardware/profiles/v2.json` selects the camera and image orientation. See [vision](../vision/README.md#tracking-modes) for mode behavior.

`vision.json` stores `vision.accelerator`: `auto`, `hailo` or `cpu`. A missing file selects `auto`. See [vision detectors](vision-reference.md#accelerator-setting) for CPU selection and backend names.

## Voice settings

`voice-settings.json` overrides startup defaults after a save. The default agent model and effort come from `agent/src/config.rs`. Speech defaults select `Qwen/Qwen3-ASR-0.6B` and `piper-alba-medium`.

Changing `voice-stack.env` does not replace saved choices. Studio's Voice settings change those choices.

## Pi voice profile

| Variable | Function |
| --- | --- |
| `ORION_STUDIO_CODEX_BIN` | Codex executable |
| `ORION_ASR_MODEL_DIR` | Qwen model directory |
| `ORION_LLAMA_SERVER` | Qwen server executable |
| `ORION_ASR_CONTEXT` | Expected vocabulary; default `Orion` |
| `ORION_ASR_THREADS`, `ORION_TTS_THREADS` | Worker threads, from 1 to 4 |
| `ORION_PIPER_MODEL_DIR` | Piper model directory |
| `ORION_PI_VOICE_URL` | Listener URL; installed value `ws://127.0.0.1:7448/` |
| `ORION_SOUL_PATH`, `ORION_MEMORY_PATH` | Personality and memory paths |
| `ORION_VOICE_HISTORY_DIR` | Shared history directory |

1. Inspect the complete service configuration before changing it.

   ```bash
   systemctl cat orion-voice-stack
   ```

2. Restart the host after changing its environment file.

   ```bash
   sudo systemctl restart orion-voice-stack
   ```

## Microphone listener

Rustpotter proposes wake candidates. The acoustic verifier checks the phrase before wake feedback. Qwen checks the complete recording before dispatching a command.

`scripts/pi_service_config.py` selects the installed wake model and threshold. `voice/models/verifier/config.json` selects the acoustic threshold. `--wake-model`, `--threshold` and `--verifier-dir` override their respective inputs. `--no-verifier` uses Qwen prefix confirmation.

| Variable | Function |
| --- | --- |
| `ORION_CAPTURE_GAIN_DB` | V1 capture gain, from 0 to 50 dB |
| `ORION_VAD_MODEL` | Silero speech detector model |
| `ORION_MIC_SPACING` | V1 microphone spacing in metres |
| `ORION_CHANNEL_SIGN` | V1 direction sign, `1` or `-1` |
| `ORION_XVF_DIRECTION` | Optional V2 USB direction diagnostics |
| `ORION_XVF_AZIMUTH_OFFSET_DEG` | USB board direction offset |
| `ORION_XVF_AZIMUTH_SIGN` | USB board direction sign |
| `ORION_XVF_FRONT_HALF_WIDTH_DEG` | Front-sector half-width, from 0 to 150 degrees |
| `ORION_XVF_MIN_ENERGY` | Minimum direction energy |

V2 wake attention uses visual speaker selection. USB direction settings do not steer that selection. Missing direction geometry disables direction diagnostics without disabling capture.

1. Restart the listener after changing `voice.env`.

   ```bash
   sudo systemctl restart orion-listener
   ```

## Runtime startup

`--character-on-start on` starts normal character movement. Installed services select it. Manual V2 serving defaults to off; V1 defaults to on.

`--rest-after-seconds` sets the inactivity interval. Its default is 1800 seconds. Lamp mode disables automatic rest. See [rest behavior](system-architecture.md#automatic-rest-and-waking).

## Systemd services

Systemd manages Linux background processes.

| Unit | Function |
| --- | --- |
| `oriond` | Hardware, movement and playback |
| `orion-listener` | Microphone and wake sessions |
| `orion-voice-stack` | Agent, coordinator and speech workers |
| `orion-studio-gateway` | Authenticated external controls |
| `orion-vision` | V2 camera and detection |
| `orion-neopixel-pin` | GPIO12 lighting setup |
| `orion-wifi-fallback.timer` | Optional network recovery checks |

The service host's discovery files live in `~/.local/share/orion/studio-service/`. This directory differs from installed release storage.

## Studio gateway

The gateway listens on port 7447. It uses a bearer token over HTTP on a trusted local network. A token grants access to its holder.

Pairing uses one spoken six-digit code. The code expires after five minutes or five failed attempts. A replacement code requires a 15-second interval.

1. Read pairing errors on the Pi when speech fails.

   ```bash
   journalctl -u orion-studio-gateway -n 60 --no-pager
   ```

2. Read the token for Studio's token-entry option when needed.

   ```bash
   cat ~/.config/orion/studio-token
   ```

## Saved pairing

Desktop Studio uses the operating system's credential store. Its service is `org.orion.studio.pairing`, with account `paired-orion`. Linux requires an unlocked Secret Service keyring. Browser development keeps pairing only in the current tab.

**Disconnect** stops reconnect attempts. **Forget Orion on this computer** removes the saved address and token.

## Weather, alerts and light routines

Weather settings contain the selected location and `celsius` or `fahrenheit`. Explicit settings take priority over memory preferences. The fallback unit is Celsius.

Clock alarms use the Pi's clock and time zone. `sunrise_minutes` accepts 5–60 whole minutes. `briefing` enables a spoken update after alarm dismissal.

`transition_seconds` accepts 0–3600 seconds. Wind-down accepts 10–60 minutes. Focus accepts 5–120 minutes. Restart cancels unfinished light transitions while preserving saved alerts.

See [alert behavior](system-architecture.md#timers-and-alarms) and [release installation](release-packaging.md).
