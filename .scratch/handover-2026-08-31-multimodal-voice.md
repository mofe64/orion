# Orion Movement Lifecycle, Multimodal Runtime, and Local Voice Handover

## Header

- **Date:** 2026-08-31
- **Status:** Movement lifecycle, physical RGBW lighting, ReSpeaker V2 audio,
  multimodal scenes, Piper speech, wake-word detection, and local command
  transcription are implemented; Orion Studio and agent/intent integration are
  next
- **Repository (workstation):** `/home/mofe/Desktop/dev/orion`
- **Repository (Raspberry Pi):** `/home/mofe/dev/orion`
- **Branch/HEAD before this handover:** `main` at `7b3c84b` (`origin/main` is
  identical)
- **Working tree before this handover:** Clean
- **Primary objective:** Record the complete functional work since the Rust
  runtime handover, its physical commissioning evidence, the current local
  voice limitations, and the proposed transition to a Mac-hosted Orion Studio
  voice/agent runtime.
- **Tags:** `orion`, `rust`, `movement-lifecycle`, `settling`, `neopixel`,
  `rgbw`, `respeaker-v2`, `audio`, `scenes`, `piper`, `wake-word`,
  `speech-to-text`, `moonshine`, `silero-vad`, `orion-studio`, `raspberry-pi`

## Continuation Update: Multimodal and Voice Foundations Now Exist

This report supersedes the continuation status in
`handover-2026-08-29-rust-runtime.md`. That report remains the authoritative
history of the Rust migration, but its statement that no LED driver, audio
integration, scene player, or voice implementation exists is no longer true.

The current implemented architecture is:

```text
                         local Unix commands
                                  |
                                  v
                       runtime/oriond (Rust)
                         /       |        \
                        v        v         v
                    motion     RGBW      ALSA playback
                   lifecycle  lighting   cues + speech
                        \        |         /
                         \       |        /
                          local scene clock

Piper Ryan Medium -> /tmp/orion-tts.sock -> oriond speech lifecycle -> ALSA

ReSpeaker microphones -> Sherpa wake -> Silero VAD -> Moonshine transcription
                                              |
                                              v
                                    /tmp/orion-wake.sock
```

The future agent runtime still does not exist. Transcripts are published but
are not interpreted or dispatched to motion, scenes, lighting, or speech.

The daemon continues to run from the source checkout during development. It is
not installed as a systemd service and does not start at boot. The only Orion
systemd unit is the narrow NeoPixel pin-configuration helper required to make
BCM12 and `/dev/ws281x_pwm` ready after reboot.

## What Was Worked On

### Movement acknowledgement and measured completion

Every accepted `goto` and `play` now receives an ephemeral daemon-local
`run_id`. The active result is stored as `motion`; the most recent terminal
result is retained as `last_motion`:

```text
executing -> settling -> completed
                      \-> timed_out
executing/settling ----> cancelled
```

- `executing` means the authored trajectory is still producing commands.
- `settling` begins after the final target has been sent.
- Completion compares final measured joint position and velocity with the
  target instead of treating elapsed trajectory time as physical success.
- All joints must remain inside both tolerances for the entire settle window.
- Default position tolerance: `0.05 rad`.
- Default velocity tolerance: `0.05 rad/s`.
- Required continuous settle duration: `0.25 s`.
- Settling timeout after trajectory completion: `2.0 s`.

These values do not restrict the commanded pose or trajectory range. They
define when the runtime is allowed to report physical completion.

`--wait` follows the exact returned run ID. Exit codes are `0` for completed,
`4` for timed out, `5` for cancelled, `3` for daemon rejection, `2` for invalid
CLI usage, and `1` for transport/runtime errors. Results are intentionally not
stored in a movement database; IDs and retained results reset when `oriond`
restarts.

The previous misleading busy-motion response was fixed. Submitting another
movement now reports `motion already active` and its `active_run_id`, rather
than claiming holding torque was not enabled.

### Physical RGBW lighting

Orion's installed light is an Adafruit NeoPixel Shield, product 2864:

- 40 SK6812-compatible RGBW pixels in a physical 5-row by 8-column matrix.
- Approximately 3000 K dedicated warm-white channel.
- BCM12 / physical pin 32 connected to shield D6/DIN.
- Physical pin 4 supplies 5 V; physical pin 30 supplies ground.
- Pi 5 RP1 PWM channel 0, 800 kHz protocol, physical GRBW wire order.

Physical mapping was commissioned as non-serpentine row-major:

```text
 0  1  2  3  4  5  6  7
 8  9 10 11 12 13 14 15
16 17 18 19 20 21 22 23
24 25 26 27 28 29 30 31
32 33 34 35 36 37 38 39
```

Confirmed points are 0 top-left, 7 top-right, 8 second-row left, and 39
bottom-right. Red, green, blue, and warm-white channel tests displayed the
expected colours. A complete low-output frame and all-off frame also passed.

`hardware/lighting/install-persistent.sh` installs the kernel-matched
`rp1_ws281x_pwm` module, overlay, module options, module-load entry, udev rule,
and `orion-neopixel-pin.service`. The physical Pi rebooted and
`hardware/lighting/verify-persistent.sh` passed. The module must be rebuilt and
reinstalled after a kernel ABI change.

`runtime/src/lighting.rs` provides the portable device boundary, recording
backend, RGBW interpolation, GRBW encoding, and `/dev/ws281x_pwm` adapter.
Direct commissioning commands are `--light`, `--light-pixel`, and
`--lights-off`. When the hardware daemon is running, it owns the device
exclusively; use scene commands rather than a second direct-light process.

### ReSpeaker V2 playback and capture

The installed board is electrically a Seeed Studio ReSpeaker 2-Mics Pi HAT V2,
not the WM8960 V1 described by the retailer copy:

- Physical codec: TLV320AIC3104 at I2C address `0x18`.
- Stable ALSA card: `seeed2micvoicec`.
- Playback and capture device: `plughw:CARD=seeed2micvoicec,DEV=0`.
- Speaker uses the JST 2.0 mono differential output fed by the right line path.
- Audio boot overlay: `respeaker-2mic-v2_0`.

The earlier WM8960 probe at `0x1a` failed with I/O error `-121`; the V1 overlay
was replaced. The persistent V2 verification passed after reboot while BCM12
remained assigned to NeoPixel PWM.

`hardware/audio/configure-playback.sh` restores the commissioned route and
PCM volume of `-6 dB`. A 440 Hz right-channel test tone was heard. The original
local `acknowledge.wav` cue and both expressive acknowledgement scenes also
played successfully on the physical JST speaker.

`hardware/audio/configure-capture.sh` selects the two single-ended microphone
inputs, disables codec AGC, and fixes PGA capture gain at `50 dB`. Testing found
that `59.5 dB` was the codec maximum but made recognition worse through noise
or clipping; `50 dB` is the commissioned value.

### Local audio cues

`runtime/src/audio.rs` adds named WAV cue validation and recording/ALSA
backends. `audio/cues/acknowledge.wav` is a tracked 420 ms ascending two-note
cue with identical stereo channels so the right-only JST route receives it.
`audio/generate_cues.py` reproducibly authors the cue with the Python standard
library.

Direct commissioning uses:

```bash
runtime/target/release/oriond --play-cue acknowledge
```

The command blocks until `aplay` exits and reports failure instead of silent
success. It must not contend with the running hardware daemon for the ALSA PCM.

### Multimodal scene runtime

`scenes/` is the portable source of truth for version-1 scenes. A scene uses an
ordered monotonic timeline and can currently:

- start a named authored motion with `play_motion`;
- move to a named pose with `goto_pose`;
- fade all 40 pixels to a logical RGBW value with `light`;
- play a named local WAV cue with `audio`.

Scenes reference semantic assets rather than raw servo, GPIO, or ALSA
commands. The library is validated before playback. Movement uses the existing
movement run ID and settling contract. Scene completion waits for every event,
the active movement, final light transition, and final audio process.

Scene runs have their own ephemeral `run_id` and retain only the active and
most recent terminal result. States are `executing`, `completed`, `timed_out`,
`cancelled`, and `failed`. `--wait` maps these to exit codes `0`, `4`, `5`, and
`6` respectively. `--stop-scene` cancels the scene and its active movement.

Tracked scenes are:

| Scene | Function |
|---|---|
| `lighting_acknowledge` | Warm acknowledgement fade, then warm-white idle; no torque required |
| `acknowledge_left` | Left expressive motion, warm light, acknowledgement cue, then idle light |
| `acknowledge_right` | Right expressive motion, warm light, acknowledgement cue, then idle light |
| `return_to_rest` | Three-second move to captured rest while fading all lights off |

Both `acknowledge_left --wait` and `acknowledge_right --wait` completed on the
assembled robot with three events dispatched and the chime heard. The normal
multimodal shutdown path is `return_to_rest --wait`, then `--disable` only
after scene completion.

### Text-to-speech selection and integration

Chatterbox Nano was implemented and benchmarked first. On the 8 GB Pi 5 it
loaded in approximately `187.65 s`, used approximately `4.37 GB` peak RSS, and
settled near a real-time factor of `4.7` after warm-up. It was too slow for live
conversation. Its approximately `2.8 GB` Hugging Face model cache and
Chatterbox dependencies were removed after Piper passed.

Piper 1.7.0 now owns synthesis in a persistent Python 3.11 worker. Orion's
selected production voice is `en_US-ryan-medium`. The physical Pi benchmark
for a 2.264-second sentence was:

- model load: approximately `1.94 s`;
- synthesis: approximately `0.299 s`;
- real-time factor: approximately `0.132`;
- peak RSS: approximately `174 MB`.

Ryan High was audible but slower at approximately `0.67` real-time factor;
Lessac Medium reached approximately `0.12`. Ryan Medium was selected for its
voice and speed. `voice/install-models.sh` installs Ryan Medium and
`voice/cleanup-voices.sh` removes other top-level Piper voices.

The worker loads Piper once and serves `/tmp/orion-tts.sock`. `oriond` owns the
temporary WAV, ALSA playback, and speech lifecycle:

```text
synthesizing -> playing -> completed
             \-> failed
synthesizing/playing -> cancelled
```

Speech commands are `--speak TEXT --wait`, `--speech-status`, and
`--stop-speech`. Only the active and most recent result are retained. Generated
WAVs are deleted after playback or cancellation and are not a speech archive.

### Pi-local wake word and command transcription

The current local listener is implemented and physically functional:

```text
ReSpeaker capture
  -> Sherpa GigaSpeech keyword detector (`HELLO WORLD`)
  -> Silero VAD command segmentation
  -> Moonshine Tiny English INT8 transcription
  -> ordered JSON-line events on /tmp/orion-wake.sock
```

`voice/install-models.sh` installs the wake, ASR, and VAD models and generates
the phrase-specific BPE token file. Click, SentencePiece, and pypinyin are
declared directly because Sherpa's token-generation CLI imports them.

`listen-worker` is the single microphone owner. It changes from wake listening
to command capture and transcription, then rearms. Captured 16 kHz mono PCM is
kept in memory and released; no microphone WAV or transcript database exists.
`wait-command` subscribes to future terminal command events.

Physical examples returned:

```json
{"event_id":2,"event":"command","state":"transcribed","text":"Return home.","audio_seconds":0.954,"inference_seconds":0.05744216600032814,"error":null}
{"event_id":4,"event":"command","state":"transcribed","text":"Hi.","audio_seconds":1.53,"inference_seconds":0.056128983000235166,"error":null}
{"event_id":6,"event":"command","state":"transcribed","text":"Look out me.","audio_seconds":0.826,"inference_seconds":0.058673225000347884,"error":null}
{"event_id":8,"event":"command","state":"transcribed","text":"He is not a judge.","audio_seconds":1.178,"inference_seconds":0.07259869300014543,"error":null}
```

This proves the end-to-end capture/VAD/ASR event path and very fast Pi
inference, but also demonstrates that wake range and transcription accuracy
are not reliable enough for the intended conversational experience. Earlier
wake tests required speaking close to the robot, and recognition degraded at
the maximum microphone gain.

The listener stops at transcript publication. It does not interpret text,
invoke a capability, move Orion, or produce a response.

## What Got Done

- Added movement run IDs without creating a movement database.
- Added executing, measured settling, completed, timed-out, and cancelled
  movement states.
- Added `--wait`, meaningful exit codes, retained last result, and corrected
  busy-motion errors.
- Added deterministic movement tests using fake feedback and controlled time.
- Commissioned the 40-pixel RGBW matrix, channel order, orientation, and
  persistent Pi 5 RP1 PWM configuration.
- Added portable recording and physical lighting backends.
- Correctly identified and persistently configured the ReSpeaker V2 codec.
- Commissioned JST speaker playback at `-6 dB` and microphone capture at
  `50 dB` with AGC disabled.
- Added a reproducible acknowledgement cue and named cue playback.
- Added a versioned local scene schema/player with motion, light, audio,
  lifecycle, cancellation, status, and wait semantics.
- Physically ran the left and right expressive acknowledgement scenes.
- Evaluated Chatterbox Nano and removed it after it failed the latency/resource
  target.
- Selected, benchmarked, and integrated Piper Ryan Medium TTS.
- Added speech IDs, synthesis/playback status, waiting, cancellation, and
  ephemeral generated-WAV cleanup.
- Added Sherpa wake detection, Silero command segmentation, Moonshine
  transcription, one microphone owner, ordered events, and fake-clock tests.
- Physically proved multiple transcriptions while identifying their accuracy
  and range limitations.

## Bugs Fixed

1. **Busy movement returned the wrong prerequisite error.** It now reports the
   active motion and `active_run_id`.
2. **Direct lighting could contend with the daemon-owned device.** Hardware
   daemon ownership and scene-based normal use are documented; the rest scene
   turns lights off through the existing owner.
3. **Pi 5 NeoPixel support vanished across reboot.** The module, overlay, pin
   mode, permissions, and verification are now persistent.
4. **Retailer documentation suggested the wrong ReSpeaker generation.** The
   electrical `0x18` codec evidence selected the correct V2 overlay.
5. **Audio volume and routing depended on ambient mixer state.** Playback and
   capture configuration scripts now restore commissioned settings.
6. **Initial `amixer` negative dB values were parsed as options.** The command
   contract uses `amixer ... -- sset` where necessary; the repeatable script
   owns the final setting.
7. **Chatterbox API/dependency and Pi performance were unsuitable.** Piper
   replaced it behind the same worker boundary.
8. **Sherpa token generation omitted runtime imports.** Click,
   SentencePiece, and pypinyin are explicit environment dependencies.
9. **Separate wake and capture processes would fight over ALSA.** The combined
   listener is now the single microphone owner.
10. **Maximum microphone gain reduced recognition quality.** Capture was fixed
    at the physically successful `50 dB` setting.

## Key Decisions and Why

### Acknowledgement does not require durable movement storage

The agent needs correlation and terminal outcome, not a movement database.
Daemon-local IDs plus active/last retention provide enough acknowledgement for
one live controller while keeping the first lifecycle slice small.

### Completion means measured arrival and settling

Elapsed duration only proves that command generation ended. Position and
velocity feedback must remain inside tolerance before a physical action can be
called complete.

### Scenes execute locally from one clock

Studio or an agent should submit semantic scenes, then observe their IDs. It
must not stream individual light frames or joint targets. Local execution
preserves synchronization if a client disconnects.

### Rust remains the sole hardware owner

Scenes, future agents, and Studio submit named capabilities. They do not write
servo registers, `/dev/ws281x_pwm`, or ALSA directly. The Python TTS process
returns audio data; Rust still owns physical playback.

### Orion remains source-run during development

Do not install `oriond`, the voice workers, or a combined Orion boot service
yet. Persistent kernel/overlay helpers configure hardware availability only.

### Piper Ryan Medium is the live Pi voice

Chatterbox quality did not compensate for a multi-second-to-tens-of-seconds
latency on this Pi. Piper is comfortably faster than real time and fits the
existing worker/speech lifecycle.

### Primary wake/STT should move to Orion Studio

The recommended next architecture is to run primary microphone capture, wake
detection, larger-model transcription, and the future conversational agent in
Orion Studio on an Apple Silicon Mac. The Pi remains authoritative for robot
capabilities and hardware status.

```text
Mac / Orion Studio
Mac microphone -> wake or push-to-talk -> VAD -> larger Whisper model -> agent
                                                                  |
                                                     semantic capability call
                                                                  |
                                                                  v
Pi / Orion                                  network API -> runtime/oriond
                                                -> motion/scenes/light/speech
```

This is a proposed continuation architecture, not implemented code. The exact
Mac hardware and memory have not been recorded, so benchmark model size before
locking the production Whisper variant. `mlx-whisper` is the preferred first
Apple Silicon backend; `whisper.cpp` remains a strong native alternative.

Wake detection remains a distinct problem from transcription. Start the first
Studio voice slice with push-to-talk so microphone capture, transcription, and
command transport can be validated independently. Add a dedicated wake model
afterward. A custom `Orion` phrase may require training rather than merely
selecting a larger Whisper model.

If Studio uses the Mac microphone, Orion hears only where that microphone can
hear. Using the robot's ReSpeaker from across the network would require a
separate 16 kHz mono PCM stream from Pi to Mac and introduces network
dependency. Keep the existing Pi listener as a diagnostic/offline fallback
until that product choice is explicit.

## Clear Next Steps

### Priority 1: Establish the Studio-to-Orion transport boundary

1. Read this report, `docs/Orion Guidebook.md`,
   `docs/orion_control_architecture.md`, `runtime/README.md`, and the existing
   scene/voice READMEs before editing.
2. Confirm the first Studio microphone is the Mac microphone. If it is the
   robot microphone, design the PCM stream explicitly before choosing a UI
   stack.
3. Define a small versioned network protocol for semantic capability requests,
   acknowledgements, and status/event subscriptions.
4. Keep `/tmp/oriond.sock` private to the Pi. Add a deliberate network-facing
   adapter or gateway; do not expose the raw Unix command grammar directly.
5. Include named `goto`, `play`, `run_scene`, `speak`, stop/cancel operations,
   run IDs, and robot status. Do not accept arbitrary servo-register writes or
   unvalidated joint streaming.
6. Add authentication or an explicit development pairing/token mechanism
   before binding beyond loopback.

The current daemon has no TCP, HTTP, or WebSocket server. This transport is the
first concrete implementation gap.

### Priority 2: Scaffold the first Orion Studio slice

1. Create the `orion_studio/` application boundary; none exists today.
2. Implement connection state, robot status, named capability submission, and
   run-ID/result display before the visual scene editor.
3. Add Mac microphone capture and push-to-talk.
4. Integrate and benchmark MLX Whisper Turbo or a larger suitable model on the
   actual Mac.
5. Display the transcript and require explicit dispatch during commissioning.
6. Add dedicated wake detection only after this path is reliable.

### Priority 3: Connect the agent runtime

1. Represent Orion actions as a bounded capability catalog backed by existing
   named poses, motions, scenes, and speech.
2. Let the agent select semantic actions and retain returned run IDs.
3. Feed terminal movement/scene/speech results back to the agent.
4. Do not let free-form model output bypass runtime validation or device
   ownership.

### Priority 4: Extend Studio into the scene authoring tool

1. Load the shared URDF/mesh, pose, motion, cue, and scene assets.
2. Add 3D pose preview, timeline editing, RGBW preview, cue preview, validation,
   YAML export, and complete-scene transfer.
3. Keep the existing format-version-1 schema as the initial interchange format
   rather than inventing a second scene representation.

### Deferred work

- Task-space pointing, forward/inverse kinematics, and target selection remain
  unimplemented.
- The depth camera is physically constrained to the base. A future perception
  slice should add its fixed transform to the URDF and calibrate camera-to-base
  extrinsics before using depth targets for pointing.
- No URDF camera joint was added in this work.
- The current scene format has no loops, per-pixel patterns, dynamic task-light
  aim, generated-speech action, or behaviour metadata yet.
- The guidebook milestone table and one paragraph in `runtime/README.md` lag
  the implementation: lighting/scenes are no longer merely planned, and the
  combined listener now does perform speech-to-text. Correct these when next
  touching those documents.

## Map of Important Files

| Path | Purpose |
|---|---|
| `runtime/src/daemon.rs` | Movement IDs, executing/settling lifecycle, measured completion, busy response |
| `runtime/src/state.rs` | Versioned runtime/movement status contract |
| `runtime/src/main.rs` | CLI, wait behavior, exit codes, hardware ownership, scene/speech commands |
| `runtime/src/lighting.rs` | Logical RGBW devices, fades, GRBW Pi 5 encoding |
| `runtime/src/audio.rs` | Cue library plus recording and ALSA playback devices |
| `runtime/src/scene.rs` | Scene schema loading, validation, clock, lifecycle, coordination |
| `runtime/src/speech.rs` | TTS worker client and generated-speech lifecycle |
| `runtime/src/socket.rs` | Local-only Unix command server/client |
| `runtime/README.md` | Current source-run operator command reference |
| `scenes/README.md` | Scene format and completion semantics |
| `scenes/*.yaml` | Four tracked physical/portable scenes |
| `audio/README.md` | Named cue conventions |
| `audio/cues/acknowledge.wav` | First tracked local cue |
| `hardware/lighting/` | Persistent RP1 PWM installation, verification, and wiring contract |
| `hardware/audio/` | ReSpeaker V2 overlay, verification, mixer playback/capture contracts |
| `voice/pyproject.toml` | Python 3.11 local voice dependencies |
| `voice/install-models.sh` | Ryan, wake, VAD, and Moonshine model installation |
| `voice/orion_voice/tts.py` | Piper Ryan Medium synthesis |
| `voice/orion_voice/worker.py` | Persistent TTS Unix-socket worker |
| `voice/orion_voice/wake.py` | ReSpeaker capture, keyword detector, event publisher |
| `voice/orion_voice/speech.py` | Silero segmentation and Moonshine transcription |
| `voice/orion_voice/listener.py` | Wake/capture/transcribe/rearm state machine |
| `voice/orion_voice/__main__.py` | `orion-voice` CLI commands |
| `voice/README.md` | Voice installation and physical test workflow |
| `docs/orion_control_architecture.md` | Current runtime, scene, and voice boundaries |
| `docs/Orion Guidebook.md` | Product roadmap and Studio architecture constraints |
| `/home/mofe/.config/orion/servo_calibration.json` (Pi only) | Authoritative physical servo calibration |

There is no tracked `orion_studio/` directory and no network-facing Orion API
at this handover.

## Test Execution Notes

The following current tests passed during handover preparation:

- **50 Rust library tests** from the current 2026-08-31 compiled test artifact,
  including deterministic movement settling/timeout, lighting, audio, scenes,
  speech, Unix sockets, and the Rust/MuJoCo integration.
- **11 Rust CLI tests** from the current compiled test artifact.
- **25 local voice tests**, including fake Piper, Sherpa wake, Silero VAD,
  Moonshine, fake-clock listener timeout, event sockets, and TTS cleanup.
- **72 motion tests.**
- **56 servo-setup tests.**
- **21 standalone MuJoCo tests** through the repository `.venv`.
- **4 robot-description tests.**
- **239 total**, no failures in the completed test runs.

Cargo was not installed in the workstation shell used to prepare this report,
so the existing current Rust test executables under `runtime/target/` were run
directly rather than rebuilding them. The Python environments were also split:
system Python held pytest, while `.venv` held MuJoCo. On a configured
development host or the Pi, rebuild with Cargo before relying on this as a
fresh-source release gate.

The first sandboxed Unix-socket test attempts failed with `Operation not
permitted`; rerunning outside that sandbox produced 50/50 Rust library and
25/25 voice passes. Those sandbox errors were environmental, not product test
failures.

## Current Raspberry Pi Workflow

Build from source:

```bash
cd /home/mofe/dev/orion
cargo build --release --manifest-path runtime/Cargo.toml --locked
```

Start the Piper worker when generated speech is needed:

```bash
voice/.venv/bin/orion-voice tts-worker
```

Start the combined local listener only when testing Pi-local wake/STT:

```bash
voice/.venv/bin/orion-voice listen-worker
```

Start the foreground hardware daemon with explicit calibration:

```bash
runtime/target/release/oriond --serve \
  --backend hardware \
  --port /dev/ttyACM0 \
  --baud-rate 1000000 \
  --calibration /home/mofe/.config/orion/servo_calibration.json
```

Configure, enable, and run a scene from another terminal:

```bash
runtime/target/release/oriond --configure
runtime/target/release/oriond --enable
runtime/target/release/oriond --run-scene acknowledge_left --wait
```

Normal multimodal shutdown:

```bash
runtime/target/release/oriond --run-scene return_to_rest --wait
runtime/target/release/oriond --disable
```

Then stop the foreground daemon with `Ctrl+C`.

Avoid running the hardware daemon under bare `sudo`: `~` would resolve to
`/root`, which previously caused a missing
`/root/.config/orion/servo_calibration.json` error. Use the development user's
device groups and explicit calibration path. If root is genuinely required,
keep every path explicit.

## Gotchas for the Next Agent

- Read this report before the older handovers; use older reports for history.
- Preserve the Rust-only, ROS-free runtime and existing scene schema.
- Do not install the foreground daemon as a system service during current
  source-run development.
- Do not confuse `orion-neopixel-pin.service` with an Orion application boot
  service. It only establishes the required RP1 pin/device state.
- The Pi calibration file is outside Git and remains authoritative.
- Run only one owner for `/dev/ttyACM0`, `/dev/ws281x_pwm`, the ALSA playback
  PCM, and the microphone capture PCM.
- Do not run `wake-worker` and `listen-worker` together.
- `wait-wake` and `wait-command` subscribe to future events; they do not query
  event history.
- Movement, scene, and speech IDs are daemon-local and ephemeral. Clients must
  retain the returned ID and observe the active/last status immediately.
- A scene returning `completed` means its motion settled, cue exited
  successfully, and light transition completed; it does not mean results were
  stored durably.
- Keep audio at the commissioned `-6 dB` playback and `50 dB` capture defaults
  unless new physical evidence justifies a change.
- Rebuild the NeoPixel kernel module after a Pi kernel ABI upgrade.
- The 40-pixel shield is powered from the Pi 5 V header and has no current
  brightness limiter in software. Continue using low values during development.
- Do not restore Chatterbox dependencies or its multi-gigabyte model cache.
- Do not interpret the fast Moonshine inference time as adequate recognition
  quality; the physical transcripts demonstrate remaining errors.
- Studio must call semantic runtime capabilities and follow run IDs. It must
  not become a second hardware controller.
- Keep the Pi listener until the Mac microphone versus robot-microphone product
  decision and offline behavior are explicit.
- Preserve unrelated worktree changes and use `apply_patch` for edits.

## Session Statistics

- Physical servos controlled: 5/5
- Physical RGBW pixels commissioned: 40/40
- Physical matrix layout: 8 columns by 5 rows, row-major
- Physical local audio cues: 1 (`acknowledge`)
- Tracked scenes: 4
- Physically completed multimodal expressive scenes: 2
- Selected TTS voice: Piper `en_US-ryan-medium`
- Pi-local wake phrase: `HELLO WORLD`
- Pi-local ASR: Moonshine Tiny English INT8
- Current Rust tests represented: 61
- Current voice tests: 25
- Full recorded test matrix: 239
- Commits after the Rust handover baseline `9371a6a`: 18
- Branch before handover: `main` at `7b3c84b`, matching `origin/main`
- Uncommitted changes before handover: 0
- Session elapsed time and token count: unknown

## Handoff Checklist

- [x] Movement IDs and lifecycle implemented
- [x] Measured position/velocity settling implemented
- [x] Movement wait/status/exit codes implemented
- [x] Busy-motion response corrected
- [x] Deterministic movement lifecycle tests added
- [x] Pi 5 RGBW hardware and persistent boot prerequisites commissioned
- [x] ReSpeaker V2 playback and capture persistently configured
- [x] Local acknowledgement cue created and physically played
- [x] Versioned local multimodal scene runtime implemented
- [x] Left and right acknowledgement scenes physically completed
- [x] Piper Ryan Medium selected and integrated
- [x] Chatterbox dependencies and cached model removed from the Pi
- [x] Wake, VAD, and command transcription implemented on the Pi
- [x] Local voice tests and physical transcript path completed
- [x] Pi-local voice reliability limitation recorded
- [ ] Define Studio network protocol and authentication/pairing
- [ ] Scaffold `orion_studio/`
- [ ] Add Mac push-to-talk and larger-model transcription
- [ ] Add reliable Studio wake-word detection
- [ ] Implement agent intent/capability routing
- [ ] Add Studio scene authoring and preview
- [ ] Decide whether the production microphone is on the Mac or streamed from Orion
- [ ] Implement task-space pointing and fixed base-camera description later

## End Matter

Orion now has a complete local functional foundation from physical movement
acknowledgement through coordinated light, motion, cue audio, generated speech,
and microphone transcription. The missing link is not another device driver:
it is the external Studio/agent boundary that turns user speech into bounded
semantic capability calls and follows their real completion.

The next agent should start by defining and implementing the smallest
Studio-to-Pi transport, then prove Mac microphone transcription with
push-to-talk. Do not remove the working Pi voice path, change the scene schema,
or expose raw hardware control while establishing that boundary.
