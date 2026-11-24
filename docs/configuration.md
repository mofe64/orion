# Orion configuration

Orion stores owner settings and personal data under the Pi user's home directory, separate from the installed release binaries so updates and rollbacks never wipe your configuration or user-authored scenes.

## Saved files

All paths below are relative to the Pi login user's home directory (`~`).

| Path | Content |
| --- | --- |
| `.config/orion/audio.json` | Master speaker volume |
| `.config/orion/voice-settings.json` | Saved agent model, effort and voice choices |
| `.config/orion/voice-stack.env` | Service executable, model and worker environment |
| `.config/orion/voice.env` | Microphone and wake settings |
| `.config/orion/routines.json` | Mode, timers, alarms and timed reminders |
| `.config/orion/routines.sounds.json` | Alarm and timer sound selections |
| `.config/orion/microphone.json` | Saved microphone mute state |
| `.config/orion/camera.json` | Camera privacy switch |
| `.config/orion/tracking.json` | Tracking mode (`off`, `idle`, `faces` or `hands`) |
| `.config/orion/vision.json` | Vision accelerator selection (`auto`, `hailo` or `cpu`) |
| `.config/orion/behaviours.json` | Optional character behavior switches |
| `.config/orion/personality.json` | Character quirks |
| `.config/orion/servo_calibration.json` | V1 servo calibration |
| `.config/orion/studio-token` | Gateway and listener bearer token |
| `.config/orion/home-assistant.json` | Private Home Assistant access token |
| `.local/share/orion/SOUL.md` | Generated personality instructions |
| `.local/share/orion/mind.json` | Runtime energy, mood, activity and attention |
| `.local/share/orion/MEMORY.md` | Saved long-term memories |
| `.local/share/orion/MEMORY.automatic.json` | Automatic-memory duplicate protection |
| `.local/share/orion/weather.json` | Weather location and temperature unit |
| `.local/share/orion/lists.json` | Shopping and to-do lists |
| `.local/share/orion/reminders.json` | Next-conversation reminders |
| `.local/share/orion/home.json` | Home memory (upcoming events, habits, recent days) |
| `.local/share/orion/home-hidden.json` | Hidden Home Assistant device discoveries |
| `.local/share/orion/voice-stack/` | Installed releases, transactions, shared models and catalogue |

If `oriond` cannot parse `routines.json` or `routines.sounds.json`, it renames the damaged file with a `.damaged` suffix, restores defaults and reports the reset in the routines `error` field.

Memory files follow `ORION_MEMORY_PATH` when set. V2 uses the tracked `hardware/profiles/v2-joints.json` file instead of a user calibration file. Built-in and user-authored assets live under `~/.local/share/orion/voice-stack/catalog/` (overrideable via `ORION_INSTALL_ROOT`), with user assets stored in `motion/user/poses/<version>/`, `motion/motions/<version>/user/` and `scenes/<version>/user/`.

## Character behaviors

`oriond` saves `mind.json` atomically (`0600`) at most once every 10 seconds and at shutdown, preserving timestamps so activity and mood decay cleanly across restarts (the agent ignores state older than 10 minutes). Studio's Debug page shows the live state via `/api/v2/status`.

`behaviours.json` stores the optional behavior switches below; missing keys use the listed defaults.

| Key | Default | Function |
| --- | --- | --- |
| `object_glances` | `false` | Look briefly at mapped desk objects |
| `mutual_gaze` | `false` | Respond to a sustained look |
| `arrival_notice` | `false` | Welcome a person after prolonged absence |
| `new_object_peek` | `false` | Look at a new object or pet |
| `presence_defers_rest` | `false` | Delay automatic rest while people or hands remain visible |
| `proactive_speech` | `false` | Say a short remark about an arrival, sustained look or new object |
| `demo_mode` | `false` | React and speak more often with larger moves and send a camera snapshot per remark |
| `mind_idle_rhythm` | `true` | Adjust idle pacing and background light warmth with energy |
| `listen_lean_in` | `true` | Lean in slightly while listening |
| `thinking_beats` | `true` | Play alternating gestures during longer thinking pauses |
| `wake_sleep_acts` | `true` | Stretch after confirmed wake and settle before rest |
| `posture_drift` | `false` | Drift slightly between resting postures during quiet idle |
| `act_sounds` | `true` | Play voice-like cues at wake, sleep, yawn, thinking, arrival, peek and wave markers |
| `move_to_music` | `sometimes` | Choose `off`, `sometimes` or `always` for dancing to music |

Legacy boolean dance settings migrate automatically (`true` → `sometimes`, `false` → `off`).

On a **fresh install**, the installer writes `behaviours.json` with `proactive_speech`, `arrival_notice`, `mutual_gaze` and `new_object_peek` enabled so Orion starts in **Lively** mode (updates keep your existing file untouched). In Studio's **Personality and memory** section, **How lively Orion is** exposes **Quiet**, **Lively** and **Demo** while preserving your underlying `proactive_speech` choice when toggling Demo off. Enabling `proactive_speech` or `demo_mode` also enables `mutual_gaze`, `arrival_notice` and `new_object_peek`, and moves `Off` or `Hands` tracking to `Idle` while leaving `Faces` unchanged; while `demo_mode` is active, vision enables all four observation switches automatically. Dance appears under **Character behaviours**, and individual switches sit under **Advanced**. See [autonomous behaviors](motion-architecture.md#autonomous-behaviors) for full rules.

## Quirks

Each lamp has two quirks in `personality.json` (`{"quirks": {"curiosity": ..., "chattiness": ...}}`), managed exclusively by the gateway:

| Key | Values |
| --- | --- |
| `curiosity` | `shy`, `normal`, `nosy` |
| `chattiness` | `reserved`, `normal`, `chatty` |

When the gateway starts and either quirk is missing or invalid, it picks an outer value (`shy` or `nosy`; `reserved` or `chatty`) at random and saves it, so every new Orion starts with its own personality. `oriond` polls this file every second and treats missing or unknown values as `normal`. See [quirks](motion-architecture.md#quirks).

## Home memory

The agent maintains `home.json` on the lamp across three sections:

- **Things to come back to:** When you mention an upcoming event (like an interview or trip), Orion quietly notes the event and date. On that day and up to 3 days afterward, the note is included with your conversation turns and Lively arrivals (never with other remarks, announcements or briefings so Orion never brings it up to an empty room). It stops once Orion asks about it, or after 3 turns without asking.
- **Habits:** Each day, the agent reads recent behavior logs to spot repeated patterns (at least 2 occurrences) such as when people first arrive, leave, turn on evening lights, reply to remarks or leave new objects on the desk.
- **Recent days:** One short summary sentence for each of the last 30 days.

Habits and the last 3 days of summaries accompany conversation turns as well as welcome, light-care and goodnight remarks. You can inspect or delete any individual item, habit or all of home memory in Studio under **Personality and memory**.

## Studio light controls

The Lamp panel in Studio stays synchronized with Orion whether you change the light by voice, in Studio or from another window. Brightness changes preserve your palette and effect, and toggling power preserves your chosen brightness and palette. Temporary voice, idle or scene lighting overrides the lamp visually without overwriting your saved settings (Studio displays a brief notice while that happens).

## Camera privacy and tracking profile

- `camera.json` stores `camera_enabled`. A missing file allows capture; an invalid or unreadable file disables capture.
- `tracking.json` stores `tracking_mode` (`off`, `idle`, `faces` or `hands`). A missing file defaults to **`faces`** (though enabling Lively/Demo switches `off` or `hands` to `idle`), and invalid files default to `off`. Camera privacy disables capture in every mode.
- Saying `"follow my hand"` or `"follow my face"` switches `tracking_mode` and remembers your prior setting in `resume_mode` so `"stop following"` restores it (Demo hand play uses the same fields). Both `oriond` and the gateway write `tracking.json` atomically (`0600`).
- `hardware/profiles/v2.json` configures the camera and image orientation. See [tracking modes](../vision/README.md#tracking-modes).
- `vision.json` selects the `vision.accelerator` backend (`auto`, `hailo` or `cpu`; missing defaults to `auto`). See [vision detectors](vision-reference.md#accelerator-setting).

## Voice settings

`voice-settings.json` overrides startup defaults from `agent/src/config.rs` (`gpt-6-luna`, effort `low`) and speech worker defaults (`Qwen/Qwen3-ASR-0.6B` and `piper-alba-medium`). Use Studio's Voice settings to update these rather than editing `voice-stack.env`.

## Pi voice profile

`voice-stack.env` sets environment variables for `orion-voice-stack`:

| Variable | Function |
| --- | --- |
| `ORION_STUDIO_CODEX_BIN` | Codex executable path |
| `ORION_ASR_MODEL_DIR` | Qwen model directory |
| `ORION_LLAMA_SERVER` | Qwen `llama-server` executable path |
| `ORION_ASR_CONTEXT` | Expected vocabulary hint (default `Orion`) |
| `ORION_ASR_THREADS`, `ORION_TTS_THREADS` | Worker threads (`1–4`) |
| `ORION_PIPER_MODEL_DIR` | Piper voice model directory |
| `ORION_PI_VOICE_URL` | Listener WebSocket URL (installed: `ws://127.0.0.1:7448/`) |
| `ORION_SOUL_PATH`, `ORION_MEMORY_PATH` | Overrides for `SOUL.md` and `MEMORY.md` |
| `ORION_VOICE_HISTORY_DIR` | Shared conversation and behavior history directory |

1. Inspect the full service configuration before editing.

   ```bash
   systemctl cat orion-voice-stack
   ```

2. Restart the service host after editing `voice-stack.env`.

   ```bash
   sudo systemctl restart orion-voice-stack
   ```

## Microphone listener

The listener uses `openWakeWord` for continuous wake detection by default (`--verifier-dir` points to `voice/models/verifier/`, where `config.json` sets threshold, startup frames and model hashes). Qwen checks a 3.3-second wake prefix before confirming, then transcribes the full command. Passing `--wake-engine rustpotter` switches back to the earlier detector (`--wake-model`, `--threshold`, and optional `--no-verifier`), and release updates preserve explicit engine choices.

`ORION_WAKE_CLIPS_DIR` (unset by default) records bounded prefix clips for debugging; see [listener troubleshooting](../voice/README.md#troubleshooting). `orion-record-voice --speaker NAME` records private training samples on the Pi; see [record your voice](wake-word-training.md#record-your-voice).

`voice.env` supports these listener variables:

| Variable | Function |
| --- | --- |
| `ORION_CAPTURE_GAIN_DB` | V1 microphone capture gain (`0–50` dB) |
| `ORION_VAD_MODEL` | Silero VAD model path |
| `ORION_MIC_SPACING` | V1 microphone spacing in metres |
| `ORION_CHANNEL_SIGN` | V1 direction sign (`1` or `-1`) |
| `ORION_XVF_DIRECTION` | Optional V2 USB direction diagnostics |
| `ORION_XVF_AZIMUTH_OFFSET_DEG` | USB board direction offset |
| `ORION_XVF_AZIMUTH_SIGN` | USB board direction sign |
| `ORION_XVF_FRONT_HALF_WIDTH_DEG` | Front-sector half-width (`0–150` degrees) |
| `ORION_XVF_MIN_ENERGY` | Minimum energy for direction estimates |

V2 wake attention selects speakers visually rather than from USB audio direction. Missing direction geometry disables direction diagnostics without interrupting capture.

1. Restart the listener after changing `voice.env`.

   ```bash
   sudo systemctl restart orion-listener
   ```

## Runtime startup

`--character-on-start on` enables character movement on boot (used by installed Pi services; manual V2 serving defaults to off, while V1 defaults to on). `--rest-after-seconds` sets the inactivity timeout (default `1800` seconds; Lamp mode disables automatic rest). See [automatic rest and waking](system-architecture.md#automatic-rest-and-waking).

## Systemd services

| Unit | Function |
| --- | --- |
| `oriond` | Hardware control, movement, lighting and playback |
| `orion-listener` | Microphone capture and wake detection |
| `orion-voice-stack` | Agent, coordinator and local speech workers |
| `orion-studio-gateway` | Authenticated Studio API and asset publication |
| `orion-vision` | V2 camera capture and local detectors |
| `orion-spotify` | Spotify Connect discovery and audio stream |
| `orion-neopixel-pin` | GPIO12 PWM pin setup |
| `orion-home-assistant-install` | On-demand root installer for Home Assistant Container |
| `orion-wifi-fallback.timer` | Optional fallback hotspot check |

The service host keeps its discovery metadata in `~/.local/share/orion/studio-service/`, separate from installed release directories.

## Studio gateway

The gateway listens on port `7447` over HTTP on your local network using bearer-token authentication (`~/.config/orion/studio-token` grants full control to its holder). Pairing uses a spoken six-digit code that expires after 5 minutes or 5 failed attempts, with a 15-second cooldown before requesting a new code.

1. Check gateway logs on the Pi if pairing speech fails.

   ```bash
   journalctl -u orion-studio-gateway -n 60 --no-pager
   ```

2. Read the bearer token directly when using Studio's manual token-entry option.

   ```bash
   cat ~/.config/orion/studio-token
   ```

Desktop Studio stores pairing credentials in your OS keyring (`org.orion.studio.pairing`, account `paired-orion`; Linux requires an unlocked Secret Service keyring, and browser dev sessions store pairing only in the active tab). **Disconnect** pauses reconnection; **Forget Orion on this computer** deletes the saved address and token.

## Weather, alerts and light routines

- **Weather:** Stores your location and `celsius` or `fahrenheit` (explicit settings override home-memory preferences; fallback is Celsius).
- **Alarms and routines:** Clock alarms follow the Pi's clock and time zone. `sunrise_minutes` accepts `5–60` whole minutes, `briefing` enables a spoken morning summary after dismissal, `transition_seconds` accepts `0–3600`, wind-down accepts `10–60` minutes, and focus accepts `5–120` minutes. Restarting `oriond` cancels active light transitions while keeping saved alerts. See [timers and alarms](system-architecture.md#timers-and-alarms).

## System metrics

The Studio gateway samples Linux `/proc`, thermal zone (`/sys/class/thermal/thermal_zone0/temp`), systemd cgroup counters and Raspberry Pi `vcgencmd` throttling flags (at most every 10 seconds), returning `null` for unavailable readings (such as Hailo temperature when unsupported). Readings are kept in memory for 15 minutes and served to Studio's System page over `/api/v2/system/metrics`.

## Home Assistant

`scripts/release-home-assistant.json` pins the `orion-home-assistant` ARM64 container image and SHA-256 digest. The container runs on host networking with a `1 GiB` memory limit, `256` CPU shares, restart-unless-stopped policy and read-only `/run/dbus` access for Bluetooth; configuration lives in `~/.local/share/orion/home-assistant` and install metadata in `~/.local/share/orion/home-assistant-install.json`.

Studio handles onboarding (using the Pi time zone and `weather.json` location, with analytics off), keeps the private Home Assistant access token on the Pi (`~/.config/orion/home-assistant.json`), and exposes **Voice control** by toggling Home Assistant's native `conversation` assistant exposure setting directly. USB Zigbee and Thread radios require manual Docker `--device` flags and are not mapped automatically.
