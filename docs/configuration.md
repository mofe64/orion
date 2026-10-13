# Orion configuration

Orion saves its settings and data on the onboard Computer in the home directory of the user account running Orion.

## Config


| File or folder                         | What it is used for                                                                                                                                                             |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `.config/orion/voice-settings.json`    | User supplied settings for Orion’s AI model, how much it thinks before replying, how it recognises speech and which voice it uses for TTS.                                      |
| `.config/orion/voice-stack.env`        | Tells Orion where to find Python, Codex and its speech model files when starting its voice system. Also sets how much CPU the speech programs can use.                          |
| `.config/orion/voice.env`              | Microphone settings, including recording volume and how Orion works out where a voice is coming from.                                                                           |
| `.config/orion/routines.json`          | Your timers, scheduled reminders and alarms, including repeating alarms and recent results. Also saves whether Orion can go to sleep automatically or stays awake in Lamp mode. |
| `.local/share/orion/weather.json`      | The location Orion uses for weather reports and whether temperatures are shown in Celsius or Fahrenheit.                                                                        |
| `.local/share/orion/lists.json`        | Your shopping and to-do lists.                                                                                                                                                  |
| `.local/share/orion/reminders.json`    | Things Orion should remind you about the next time you talk to it.                                                                                                              |
| `.config/orion/routines.sounds.json`   | The alarm and timer sounds you’ve chosen.                                                                                                                                       |
| `.config/orion/microphone.json`        | Whether Orion’s microphone is muted. The choice is remembered after a restart.                                                                                                  |
| `.config/orion/camera.json`            | Whether Orion is allowed to use its camera.                                                                                                                                     |
| `.config/orion/behaviours.json`        | Optional behaviours you’ve enabled, such as noticing someone arrive or glancing at objects.                                                                                     |
| `.config/orion/tracking.json`          | Whether Orion follows faces, follows hands, keeps the camera available without continuously following anyone, or turns tracking off.                                            |
| `.config/orion/servo_calibration.json` | The motor positions Orion uses as reference points, which way each motor turns and how far each joint is allowed to move.                                                       |
| `.config/orion/studio-token`           | Access key used to let Studio and the voice service connect to Orion.                                                                                                           |
| `.local/share/orion/SOUL.md`           | The personality and speaking style you’ve chosen for Orion.                                                                                                                     |
| `.local/share/orion/MEMORY.md`         | Facts and preferences Orion remembers between conversations.                                                                                                                    |
| `.local/share/orion/voice-stack/`      | Installed Orion software, downloaded models and tools, and records of software updates.                                                                                         |


Orion stores its assets (poses, movements, scenes and sound files) in a project folder on the onboard computer. The deployment flag`--root` sets this folder.

The programs that run Orion (voice, runtime, vision) are installed separately under `~/.local/share/orion/voice-stack/releases/`. Updating Orion switches to the new programs and updates the built-in poses, movements and scenes in the project folder. 

User authored poses are saved in `motion/user/poses/<version>/`, movements in `motion/motions/<version>/user/`, and scenes in `scenes/<version>/user/`, all inside the project folder, and all software updates preserve these folders.

## Character behaviours

`~/.config/orion/behaviours.json` stores boolean keys  for `object_glances`, `mutual_gaze`, `arrival_notice`, `new_object_peek` and `presence_defers_rest` behaviour config settings.  
Missing files or keys mean are treated as false bool flags and disables all behaviours. 

## Camera privacy and tracking profile

`camera_enabled` in `~/.config/orion/camera.json` enables and disables video capture via orion's camera If the file is missing, we deault to enabled. 

`tracking_mode` in `~/.config/orion/tracking.json` controls which specific tracking profile orion uses.

We have four options `off`, `idle`, `faces` or `hands`. A missing file defaults to `faces`; unreadable, corrupt or
unknown settings select `off`.

`hardware/profiles/v2.json` is the built-in description of Orion V2’s hardware. Its camera settings tell Orion which camera to use and whether to flip the picture. The installer uses this file to include the camera software for V2.

Note: V1 of orion does not have a camera, so does not use camera software

## Voice settings

For Orion we default to `gpt-6-luna` with `medium` effort for intelligence, `Qwen/Qwen3-ASR-0.6B` for ASR and `piper-alba-medium` for TTS.  
For our ASR, we ask Qwen to transcribe in English rather than auto-detecting the language.  This means Orion replies in English unless the user explicitly asks to switch.

Orion uses the speech settings saved in `~/.config/orion/voice-settings.json`. If that file does not exist, it uses the default speech models and folders from its startup setup. Changing startup defaults in `voice-stack.env` does not replace choices you’ve already saved. Use Studio to change your saved voice settings.

## Pi voice profile

`~/.config/orion/voice-stack.env` sets up Orion’s speech and AI programs. Restart `orion-voice-stack` after changing it. 

`~/.config/orion/voice.env` controls microphone capture and voice direction. Restart `orion-listener` after changing it.

Systemd, Linux’s service manager, can also set these values in service files and `.service.d/*.conf` files. Run `systemctl cat SERVICE` to see a service’s startup settings before editing them.


| Variable                                 | What it does                                                                                                                     |
| ---------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `ORION_STUDIO_CODEX_BIN`                 | Path to the Codex program that runs Orion’s AI agent. The installer selects its installed copy.                                  |
| `ORION_ASR_MODEL_DIR`                    | Folder containing the Qwen speech recognition files, `model.gguf` and `mmproj.gguf`.                                             |
| `ORION_LLAMA_SERVER`                     | Path to the program that runs the Qwen speech recognition model.                                                                 |
| `ORION_ASR_CONTEXT`                      | Words Qwen should expect to hear, helping it recognise names. Defaults to `Orion`.                                               |
| `ORION_ASR_THREADS`, `ORION_TTS_THREADS` | How many CPU threads speech recognition and speech generation can use. Accepts 1–4; the installer sets both to 3.                |
| `ORION_PIPER_MODEL_DIR`                  | Folder containing the files for Orion’s speaking voice. Defaults to `~/.local/share/orion/voice-stack/models/piper-alba-medium`. |
| `ORION_PI_VOICE_URL`                     | Address the voice service uses to connect to the microphone listener. Defaults to `ws://127.0.0.1:7448/` on the Pi.              |
| `ORION_SOUL_PATH`, `ORION_MEMORY_PATH`   | Choose different files for Orion’s personality and saved memories.                                                               |


If `ORION_STUDIO_CODEX_BIN` is set, Orion uses that program and does not search
for another copy. Otherwise, it looks in supported installed apps, then for the
command-line program on `PATH`. Orion checks that Codex is logged in and which
AI models and thinking levels it supports. 

## Systemd services that run on orion's onboard computer

Systemd, starts and manages the following Orion’s background programs:

- `orion-voice-stack` runs speech recognition, the AI agent and spoken replies.
- `orion-listener` listens to the microphone and detects our wakeword (“Hey Orion”)
- `orion-studio-gateway` lets Studio connect to Orion.
- `oriond` controls Orion’s motors, lights and sound playback.
- `orion-vision` runs the camera and visual detection on V2 hardware.
- `orion-neopixel-pin` prepares the light’s control pin when the onboard computer starts.
- `orion-wifi-fallback` restores Wi-Fi or starts Orion’s hotspot when disconnected. It is optional.
- `orion-wifi-fallback.timer` periodically runs the optional Wi-Fi check.

The voice stack service will retry its conversation process every five seconds if it stops, and Closing Studio leaves the listener running, but respects the mic disabled / enabled flag.

Orion automatically stores the files used by the gateway and voice service to
communicate in `~/.local/share/orion/studio-service/`.

## Studio gateway

The gateway runs on Orion’s onboard computer. It lets Studio change Orion’s
settings and control its movements, lights and sound.

To connect Studio:

1. Enter Orion’s gateway address and choose **Get code**.
2. Orion reads a six-digit code aloud twice. Enter the code and choose **Pair**.

Studio receives Orion’s access key and uses it for future requests. The desktop
app saves the connection so you can reconnect later. See [saved pairing](#saved-pairing).

The code works once and expires after five minutes. Five wrong attempts cancel
it. **Say it again** gives you a new code; wait fifteen seconds between requests.

If Orion cannot say the code, read it and the speech error in the gateway log on
the onboard computer:

```bash
journalctl -u orion-studio-gateway
```

You can also connect using Orion’s access key directly. Read it on the onboard
computer:

```bash
cat ~/.config/orion/studio-token
```

In Studio, choose **Use a token instead** and enter that key.

## Saved pairing

Desktop Studio saves Orion’s address and access key in the computer’s secure
credential store, under `org.orion.studio.pairing` with account `paired-orion`.
This is Keychain on macOS, Credential Manager on Windows and Secret Service on
Linux. Linux needs an unlocked keyring. When running Studio in a browser for
development, pairing lasts only for the current tab.

If the connection drops, Studio retries with delays from one to fifteen seconds.
Each connection request waits up to five seconds. If Orion rejects the access
key, Studio asks you to pair again. **Disconnect** stops retries for that session.
**Forget Orion on this computer** removes the saved address and key.

## Deployment

When deploying we can use the folloing env vars or flags, when deploying to the onboard computer.  Command-line options take priority over environment variables.


| Variable            | What it chooses                                                 | Default                    | Command-line option                |
| ------------------- | --------------------------------------------------------------- | -------------------------- | ---------------------------------- |
| `ORION_PI_HOST`     | The onboard computer to connect to, including its user account. | `mofe@ariadne-robot.local` | `--host USER@HOST`                 |
| `ORION_PI_ROOT`     | Orion’s project folder on the onboard computer.                 | `/home/mofe/orion`         | `--root PATH`                      |
| `ORION_PI_BRANCH`   | The Git branch containing the software to install.              | `main`                     | `--branch BRANCH`                  |
| `ORION_PI_HARDWARE` | The robot’s hardware version.                                   | Required                   | `--hardware v1` or `--hardware v2` |


Commit and push any of your runtime changes  before deploying. 

The script builds and tests the latest pushed commit from the selected branch locally using Docker, then installs it on Orion’s onboard computer.

deployment replaces assets (built-in poses, movements and scenes) with the latest version on the deploy branch. It also preserve user authored assets, saved settings, calibration, access key and custom startup settings.

The deployment process moves Orion to check its movements, lights and sound, and finishes in the calibrated rest position with the lights off and motors unpowered.

## Customising the oriond runtime

`oriond` controls Orion’s motors, lights and sound.

`--character-on-start on` starts Orion’s normal movement and light behaviours.
`off` starts with the motors unpowered. Installed services use `on`. When
starting `oriond` by hand, V1 defaults to `on` and V2 defaults to `off`.

`--rest-after-seconds` sets how long Orion can be inactive before resting.
The default is `1800` seconds, or 30 minutes. A confirmed “Hey Orion” resets the
timer. Conversations, unfinished actions and enabled camera behaviours can delay
rest. Lamp mode keeps Orion awake. See
[automatic rest](system-architecture.md#automatic-rest-and-waking).

Orion saves its Idle/Lamp mode, timers, reminders and alarms in
`~/.config/orion/routines.json`, and sound choices in
`~/.config/orion/routines.sounds.json`. Updates preserve both files.

## Microphone listener

`orion-listener` listens to the microphone and checks for “Hey Orion”.

### Wake word

The listener checks “Hey Orion” twice to reduce false wake-ups:

1. Rustpotter looks for a match using `voice/models/wake/hey_orion_reference.rpw`,
  with a threshold of `0.35`.
2. openWakeWord checks that match using `voice/models/verifier/`, with a threshold
  of `0.5`.

A threshold sets how strong a match must be to pass. `--threshold VALUE` changes  
the first check. The second is set in  
`[voice/models/verifier/config.json](../voice/models/verifier/config.json)`.  
`--verifier-dir PATH` chooses a different model for the second check.  
`--no-verifier` uses Qwen to transcribe the audio and check the wake phrase instead.  
Updates keep thresholds you have chosen yourself.

### Microphone settings

Save these in `~/.config/orion/voice.env` and restart `orion-listener`.


| Variable                | What it does                                                                                                                    |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `ORION_CAPTURE_GAIN_DB` | Sets V1 microphone recording gain from 0–50 dB. The installed default is 25 dB.                                                 |
| `ORION_VAD_MODEL`       | Points to the Silero model file that detects when someone is speaking.                                                          |
| `ORION_MIC_SPACING`     | Distance between V1 microphones, in metres. Defaults to `0`, which disables voice direction.                                    |
| `ORION_CHANNEL_SIGN`    | Use `1` or `-1` to match the V1 microphone channels to Orion’s left and right. Defaults to `0`, which disables voice direction. |


V2 gets voice direction from its XVF3800 microphone board:


| Variable / listener option                                      | Default | What it does                                                                                            |
| --------------------------------------------------------------- | ------- | ------------------------------------------------------------------------------------------------------- |
| `ORION_XVF_DIRECTION` / `--xvf-direction`                       | `0`     | Set to `1` to enable voice direction on V2.                                                             |
| `ORION_XVF_AZIMUTH_OFFSET_DEG` / `--xvf-azimuth-offset-deg`     | unset   | Corrects for the angle at which the microphone board is mounted, in degrees. Measure this on the robot. |
| `ORION_XVF_AZIMUTH_SIGN` / `--xvf-azimuth-sign`                 | unset   | Use `1` or `-1` so the board’s readings match Orion’s left and right.                                   |
| `ORION_XVF_FRONT_HALF_WIDTH_DEG` / `--xvf-front-half-width-deg` | `30`    | Angle either side of straight ahead that counts as “in front”. Accepts 0–150 degrees.                   |
| `ORION_XVF_MIN_ENERGY` / `--xvf-min-energy`                     | `0`     | Ignores direction readings when speech strength is at or below this value.                              |


Without the measured mounting angle and direction sign, V2 voice direction stays off. It also turns off if the board cannot be read; microphone recording still works. 

Systemd starts the listener with `oriond`. Stopping or restarting `oriond` does
the same to the listener.

## Weather

`weather.json` is saved beside the configured `MEMORY.md`, normally at
`~/.local/share/orion/weather.json`.


| Setting            | What it does                                                                                                                       |
| ------------------ | ---------------------------------------------------------------------------------------------------------------------------------- |
| `location`         | The place selected in Studio, including its name, coordinates and time zone.                                                       |
| `temperature_unit` | Choose `celsius` or `fahrenheit`. If unset, Orion uses your saved temperature preference from memory, or Celsius if you have none. |




## Alarm settings

Alarms follow the onboard computer’s clock and time zone. Make sure these are
correct.


| Setting           | What it does                                                                                                   |
| ----------------- | -------------------------------------------------------------------------------------------------------------- |
| `briefing`        | Set to `true` for a spoken update after you dismiss the alarm. Defaults to `false`.                            |
| `sunrise_minutes` | Gradually brightens the light before the alarm. Choose 5–60 whole minutes, or leave it unset to turn this off. |




## Light settings


| Setting              | What it does                                                                                                    |
| -------------------- | --------------------------------------------------------------------------------------------------------------- |
| `transition_seconds` | Time for a brightness or colour change, from 0–3600 seconds. The default of `0` applies the change immediately. |
| `wind_down`          | Gradually dims the light before Orion rests. Choose 10–60 minutes.                                              |
| `focus`              | Uses a brighter light with a timer. Choose 5–120 minutes.                                                       |


Restarting Orion cancels an unfinished light transition or light routine, but
keeps the focus timer. These light routines do not change your saved Idle/Lamp
mode.