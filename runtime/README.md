# Orion Rust runtime

`oriond` owns Orion's movement, RGBW light and speaker playback. It loads poses
and motions, compiles smooth joint trajectories, applies calibration, checks
measured completion and coordinates character behavior, scenes and speech
animation. The hardware and MuJoCo backends use the same 50 Hz runtime loop.

See the [system architecture](../docs/system-architecture.md) for
workstation/Pi boundaries and device ownership.
Select `--hardware v1` or `--hardware v2` for the lamp interfaces and asset paths.
The [hardware-version guide](../docs/hardware-versions.md) defines the v2 joint
mapping, maintenance defaults and calibration paths.

For movement internals, use:

- [How Orion moves](../docs/motion-architecture.md)
- [Motion reference](../docs/motion-reference.md)

The physical transport uses
[`rustypot`](https://github.com/pollen-robotics/rustypot) for the STS3215
serial packet format, synchronized reads/writes, and communication. This
servo-wire protocol is unrelated to Orion's asset format. Orion retains its
own raw register map and conversions for firmware bytes at addresses 0/1, a
one-byte maximum-acceleration register at address 85, and project-specific
encoder/velocity conversions.

## Capture-time camera rays (V2)

`camera-rays` is a pure Unix-socket query; it never enables torque or moves
Orion. Start the daemon from the catalog root (the systemd working directory).
For `--hardware v2`, including the V2 MuJoCo backend, startup loads
`description/urdf/orion-v2.urdf` once. A missing or invalid model disables this
query with its cause in the error response and startup log; other commands
remain available. V1 does not load camera geometry.

```text
camera-rays {"capture_boottime_ns":123456789000,"rays":[[0,0,1]]}
{"ok":true,"bearings":[{"azimuth":0.1,"elevation":-0.2}],"pose_age_ms":40.0}
```

Rays use the camera's ROS optical axes (+X right, +Y down, +Z forward), must be
finite unit vectors, and number 1–16. Bearings are radians in the base frame:
azimuth zero is ahead (−Y), positive is the lamp's right (−X), and positive
elevation is up. Measured calibrated angles enter FK directly.

After each successful feedback read, the 50 Hz tick reads
`clock_gettime(CLOCK_BOOTTIME)` and copies five measured angles into a
64-slot array retaining at most one second. This write allocates nothing.
Queries linearly interpolate neighbouring samples; they reject captures older
than the retained buffer or more than 50 ms ahead of the boot clock. Interpolation
requires bracketing feedback samples at most 50 ms apart; exact recorded samples
remain usable at the edges of an outage. A capture
slightly ahead of the last measured sample uses that sample, which must be
no more than 50 ms old at query time.
`pose_age_ms` is the age of the pose used (the interpolated capture time, or
the last measured time for tolerated future captures). Missing or stale
feedback produces an error rather than an invented pose.

Linux camera and joint timestamps include suspend; `Instant` is unsuitable
for this alignment. macOS simulation uses the equivalent continuous boot
clock, with its timebase cached at startup. Model loading and query parsing
run outside `RuntimeCore::tick`; the query accepts at most 16 rays through
the existing bounded socket command handling.

Software tests validate interpolation and stationary-target stability during
a yaw/swivel sweep. Physical timestamp-to-joint alignment and fitted camera
mount accuracy remain unverified on the lamp.

## Bearing gaze (V2)

V2 servers enable gaze by default on hardware and MuJoCo. V1 and non-serve
operations leave it off; explicit `--gaze on` requires a V2 server.
Use `--gaze off` to disable it. V2 still defaults to character mode off for
maintenance; enable character mode explicitly for a powered preview:

```bash
runtime/target/debug/oriond --serve --backend mujoco --hardware v2 \
  --start-pose home --character-on-start on
```

The socket accepts these commands; `capture_boottime_ns` uses the same clock
as `camera-rays`, and each target's timestamp must increase:

```text
gaze target {"track_id":1,"capture_boottime_ns":123456789000,"azimuth":0.4,"elevation":0.0,"confidence":0.95,"class":"face"}
gaze status
gaze clear
gaze resume
```

Targets contain bearings, never joints. Optional `range` is positive metres;
orientation gaze does not use it to move the arm. `gaze status` reports enabled
state, track and run IDs, tracking/settling state, clamping, extension count, error, and the compiled peak
and base/neck planning-step speeds. Clamping compares the bounded aim pose
with the requested bearing, including steps that have yet to reach the target.
It also reports a base yaw restricted by calibration, even when the neck
compensates and the beam reaches the bearing.
An unavailable powered character, foreground work or speech drops the target
with `ok:true, accepted:false`; an invalid, stale or disabled request is rejected.

Gaze replaces one queued target and extends an anchor-relative character
performance at most 10 Hz. The neck leads for 200 ms, the base catches up when
the swivel leaves a 4° centre deadband, and wrist pitch handles elevation.
URDF FK solves the tilted swivel geometry. Both models use calibrated angles
directly. Every extension uses the existing spline compiler, calibration and
settle checks, and ends at the immutable anchor.

Neck planning steps use 2.5 rad/s and base catch-up steps use 0.6 rad/s;
these size keyframe increments. The compiler preview limits every joint to
a shared peak of 1.5 rad/s,
including the return. A moving handover above that speed waits rather than
discarding measured velocity. Targets older than one second stop extensions;
the last plan holds for up to another 1.1 seconds and settles. `gaze clear` shortens that hold.
Scenes, direct foreground movement, speech, rest and Stop preempt gaze.
Stop and disable latch gaze off: subsequent targets cannot restart it.
Status shows `state:"stopped"` with `stop_command:"stop"` or `"disable"`.
A confirmed current-session wake clears a Stop latch only when character mode
is enabled, startup has finished and holding torque is already on. This recovery
never starts home or enables torque; powered scene or speech work retains control.
After disable, use `character start` to restore torque, start home and clear the
latch. Wake cannot clear a disable latch, even after manual `enable`.
`gaze resume` refuses while torque is off. Scripts can explicitly `enable` then
`gaze resume`; resume only clears the latch, without starting home or arming rest.
Successful recovery sets `stop_command` to `null`. A later Stop cannot downgrade
a disable latch.
Targets, `gaze clear`, idle time, returning vision and stale wake sessions cannot
clear Stop. None of these recovery requests override `--gaze off`.

If camera geometry or its boot clock fails to load, the daemon keeps serving.
Gaze reports `enabled:false, state:"unavailable"` with the startup error, and
`camera-rays` is disabled. Restore the URDF and its frame references, then restart
the daemon; resume requests cannot repair missing geometry. Explicitly disabled
gaze reports `state:"off"`.

Native MuJoCo checks tracking and software ownership. These are
`simulation_preview` performances: physical tracking, safe cable motion,
speed/deadband tuning and camera alignment remain unverified on the lamp.

## Build and test

Use a toolchain that supports the crate's Rust 2024 syntax. Run from the repository root:

```bash
cargo build --manifest-path runtime/Cargo.toml
cargo test --manifest-path runtime/Cargo.toml --all-targets --locked
cargo fmt --manifest-path runtime/Cargo.toml --check
cargo test --manifest-path runtime/Cargo.toml --doc --locked
python3 -m unittest discover -s runtime/tests -p 'test_*.py' -v
```

Component tests live beside their Rust implementations. Rest policy tests use
explicit clock values and injected driver failures; app tests check command
validation and session ownership without reproducing the server loop.

Integration tests live in `tests/` and send commands to the built `oriond` through
private temporary Unix sockets. `test_daemon.py` covers startup, maintenance,
command handling, and movement/scene completion. `test_rest.py` covers automatic
rest, confirmation during descent, attention freshness, speech ordering, and
fault handling through the real server loop. Both share process and socket
helpers in `daemon_support.py` and use recording audio and lighting.

The deterministic following driver verifies software coordination, including
injected stalled movement and torque-release failure. It does not model physics.
Native MuJoCo tests use the tracked model and expect the
[simulator Python environment](../simulation/mujoco/README.md#python-environment)
at `.venv/bin/python`. Physical rest/wake acceptance remains separate;
see the [rest/wake integration tests](tests/test_rest.py).
Set `ORION_TEST_BIN_DIR` to an absolute binary directory to test a release build.

## Opt-in servo tracking

Pass `--tracking-log FILE` with `--serve` to capture exact trajectory goals and
measured feedback without blocking on file I/O. The file must not exist.
See [servo tracking capture](../docs/servo-tracking.md) for timing semantics,
repeatable idle/speech trials, analysis commands and measurement limitations.

## Experimental servo gains

Hardware measurements can select one arm's P/I coefficients with
`--experimental-servo-gains shoulder_pitch_joint:24:0` or
`--experimental-servo-gains elbow_pitch_joint:40:0`. This flag requires
`--serve --backend hardware --tracking-log FILE`. It accepts P from 1 to 254
and I from 0 to 1, rejects repeated selections and other joints, and preserves D.
Without the flag, the commissioned profiles are unchanged.

The coefficients are EEPROM settings, not process-local servo state. The driver
writes only differences, but ending the daemon does not itself restore them.
Use a temporary service, re-apply the original profile while torque is off, and
read back P/D/I and the EEPROM lock before returning to ordinary operation.
See [gain experiment procedure](../docs/servo-tracking.md#gain-experiments).

## Deploy an update to the Raspberry Pi

Use the [Pi deployment procedure](../docs/quickstart.md#deploy-to-the-pi) from the
workstation after committing and pushing the intended code. It prepares all four
services in a separate release, updates built-in YAML after confirmed mechanical
rest, and runs a physical smoke test before returning to rest with lights and
torque off. The [configuration reference](../docs/configuration.md#raspberry-pi-deployment)
explains saved settings, overrides and the existing asset catalog.

For logs, readiness checks and rollback, follow
[recovery](../docs/quickstart.md#logs-and-recovery). Normal character startup can
move Orion home. The default inactivity interval is documented under
[automatic rest](../docs/system-architecture.md#automatic-rest-and-waking).

## MuJoCo daemon

Run the daemon without opening a serial port:

```bash
runtime/target/debug/oriond --serve --backend mujoco \
  --start-pose attentive --character-on-start off
```

In another terminal, use the normal client commands:

```bash
runtime/target/debug/oriond --status
runtime/target/debug/oriond --configure
runtime/target/debug/oriond --enable
runtime/target/debug/oriond --goto home --duration 3.0 --wait
runtime/target/debug/oriond --play look_at_left_expressive --wait
runtime/target/debug/oriond --stop
runtime/target/debug/oriond --disable
```

Use `--socket`, `--scene`, `--python`, or `--start-pose` to override the
defaults. The MuJoCo bridge reports measured positions and velocities and
accumulates the shared base translation, tilt, height, and contact policy in
`motion/config/stability_limits.yaml`.

## Physical hardware

The Rust transport has been validated on Orion's Raspberry Pi and five-servo
STS3215 bus. Managed deployment selects binaries from a release directory while
keeping the existing catalog root for poses, motions, scenes and cues. Inspect
`systemctl cat oriond` to find the installed executable and arguments.

Servo configuration writes are checked by reading the register back. If a write
acknowledgement times out, configuration continues only when that read confirms
the exact requested value; it does not resend the write. A mismatched value or
failed read-back aborts configuration before torque is enabled. Errors identify
the servo and register. This recovery applies to profile settings, while torque
and EEPROM lock commands still require their acknowledgements.

The examples below use a runtime built in the development checkout. Stop the
installed `oriond.service` before opening its serial, RGBW or audio devices with
a direct diagnostic process. For a manual configure/enable sequence, start with
`--character-on-start off` so automatic startup does not move the robot first.

### Read hardware state without enabling torque

```bash
cd /home/mofe/dev/orion

runtime/target/release/oriond --check \
  --port /dev/ttyACM0 \
  --calibration /home/mofe/.config/orion/servo_calibration.json
```

`--check` reads one direct state snapshot and exits. It does not enable torque
or write servo registers.

### Start the runtime

In Terminal 1:

```bash
cd /home/mofe/dev/orion

runtime/target/release/oriond --serve \
  --backend hardware \
  --port /dev/ttyACM0 \
  --baud-rate 1000000 \
  --character-on-start off \
  --calibration /home/mofe/.config/orion/servo_calibration.json
```

The expected startup message is:

```text
oriond: observing hardware at 50 Hz on /tmp/oriond.sock
```

Leave Terminal 1 running. The foreground daemon owns the serial connection and
serves commands through `/tmp/oriond.sock`.

### Control Orion

Open Terminal 2:

```bash
cd /home/mofe/dev/orion

runtime/target/release/oriond --status
runtime/target/release/oriond --configure
runtime/target/release/oriond --enable
runtime/target/release/oriond --status
```

Run a named pose:

```bash
runtime/target/release/oriond --goto home --duration 3.0 --wait
```

Run an authored movement:

```bash
runtime/target/release/oriond --play look_at_left_expressive --wait
```

Movement submission is asynchronous unless `--wait` is present. Every accepted
`goto` or `play` receives a daemon-local `run_id` and follows this functional
lifecycle:

```text
executing -> settling -> completed
                      \-> timed_out
executing/settling ----> cancelled
```

`executing` means authored trajectory frames are still being sent. `settling`
begins after the final target is sent and compares measured joint position and
velocity against the completion tolerances. The measured state must remain
within tolerance for the full settle duration. The defaults are
`0.05 rad`, `0.05 rad/s`, `0.25 s` settled, and a `2.0 s` settling timeout.

Status JSON keeps only the active `motion` and the most recent terminal
`last_motion`. Clients retain the returned `run_id` and follow that ID through
these fields. IDs reset when the daemon restarts.

`--wait` is a thin client over the same status contract. It exits `0` for
`completed`, `4` for `timed_out`, and `5` for `cancelled`. Daemon command
rejection exits `3`, invalid CLI usage exits `2`, and transport/runtime errors
exit `1`.

Stop the active movement and hold its commanded position:

```bash
runtime/target/release/oriond --stop
```

### Normal shutdown

Move Orion to its captured mechanical rest pose before disabling torque:

```bash
runtime/target/release/oriond --goto rest --duration 3.0 --wait
```

Once the rest run reports `completed`, disable torque:

```bash
runtime/target/release/oriond --disable
```

Then stop Terminal 1 with `Ctrl+C`.

The normal hardware lifecycle is `--serve`, `--configure`, `--enable`, motion
commands, `--goto rest --wait`, confirmed completion, and finally `--disable`.
Neither `--disable` nor stopping the daemon is a physical emergency stop; an
accessible hardware torque/power interruption remains required during physical
trials.

## Runtime structure

| Location | Responsibility |
| --- | --- |
| `src/main.rs` and `src/lib.rs` | Thin executable entry point and public library exports. |
| `src/app/options.rs` | CLI arguments, defaults, help, and validation. |
| `src/app/client.rs` | Command submission, movement/scene waiting, and exit codes. |
| `src/app/server.rs` | Startup, device wiring, signal handling, and the 50 Hz loop. |
| `src/app/commands.rs` | Voice, lamp, character, speech, scene, movement, and reload dispatch. |
| `src/control/` | Runtime core, movement ownership, measured completion, and status. |
| `src/motion/` | Pose loading, motion library, trajectory compilation, styles, and calibration. |
| `src/devices/driver.rs` | Shared `RuntimeDriver` and `JointLimit` contracts. |
| `src/devices/sts3215/` | Physical servo driver, profiles, registers, and transport. |
| `src/devices/mujoco.rs` and `mujoco_bridge.py` | Simulator driver and its Python worker. |
| `src/devices/audio.rs` and `src/devices/lighting.rs` | Local sound and RGBW device implementations. |
| `src/expression/` | Character behavior, scenes, speech, voice feedback, and lamp programs. |
| `src/expression/rest.rs` | Inactivity deadline, rest/wake completion, voice readiness, and light gating. |
| `src/ipc/socket.rs` | Private Unix command transport. |
| `src/bin/orion-trajectory.rs` | Shared trajectory export executable. |

`CharacterCoordinator::preempt_idle_or_thinking()` interrupts either tracked
background movement. Generic movement code uses the shared device interface,
while the STS3215 and MuJoCo implementations provide their own I/O.
See [the speech animation runtime](../docs/speech-animation-runtime.md) for planning and execution.

## Lighting, audio, and local scenes

The physical light adapter selects the v1 40-pixel matrix or v2 24-pixel ring on
Pi 5 BCM12. After installing and reboot-verifying the persistent RP1
pulse-width modulation (PWM) setup described in `hardware/lighting/README.md`,
direct output is available without starting the servo daemon:

```bash
runtime/target/release/oriond --light 8 3 0 20
runtime/target/release/oriond --light-pixel 0 0 0 0 8
runtime/target/release/oriond --lights-off
```

Arguments are logical `RED GREEN BLUE WHITE` bytes from 0 through 255. The
adapter performs the physical green-red-blue-white (GRBW) ordering and 800 kHz
symbol encoding. This
path has been tested on the physical robot, including all four channels,
the full matrix, and all-off output.

The v1 physical audio adapter uses the stable Advanced Linux Sound Architecture
(ALSA) pulse-code modulation (PCM) device
`plughw:CARD=seeed2micvoicec,DEV=0`. It applies the confirmed ReSpeaker V2 JST
mixer route whenever the hardware daemon starts. Named, local, stereo WAV
cues live under `audio/cues/` and can be checked without starting the
servo daemon:

```bash
runtime/target/release/oriond --play-cue acknowledge_warm
```

V2 selects `plughw:CARD=Array,DEV=0`, skips the HAT mixer route and sets the
XVF3800 playback controls to 0 dB instead. Add `--hardware v2` to direct light
or cue commands for that lamp.

The direct command blocks until `aplay` exits and returns nonzero if playback
fails. Do not run it concurrently with a hardware daemon that may also own the
ALSA PCM.

Direct warm-cue playback and the complete `acknowledge_left` and
`acknowledge_right` motion/light/audio scenes use the same audio
path as the character coordinator.

Portable scenes live under `scenes/v1/` or `scenes/v2/`, selected by `--hardware`.
The `format_version: 2` schema coordinates
non-overlapping motion clips with parallel spatial
RGBW effects and queued audio. Events use seconds or Rust-compiled motion
markers from one supplied monotonic clock. Scene files are validated against
the pose, motion, effect, and cue libraries before playback.

The scene player implements `SceneMotionDevice` for `RuntimeCore`, so it starts
motion through the existing `goto`/`play` command boundary and follows the
returned movement `run_id`. A scene remains active while movement is executing
or settling, while an audio cue is playing, or while a light transition is in
progress. It propagates movement timeout, cancellation, and failed WAV player
exit status.

Hardware `--serve` opens `/dev/ws281x_pwm`, clears it to establish a known
initial state, configures the selected hardware's mixer, and owns both devices until the
process exits. Direct lighting and cue commands therefore should
not run concurrently with the daemon. MuJoCo uses recording lighting and audio
backends with the identical scene clock and lifecycle.

With the daemon running, test light and audio without enabling torque:

```bash
runtime/target/release/oriond --run-scene deployment_smoke --wait
runtime/target/release/oriond --scene-status
```

After `--configure` and `--enable`, run the coordinated motion, light, and
audio scene:

```bash
runtime/target/release/oriond --run-scene acknowledge_left --wait
```

Every accepted scene receives a daemon-local `run_id`. `--scene-status` keeps
only the active `scene` and most recent terminal `last_scene`; IDs and results
reset when the daemon restarts. Scene states are `executing`,
`completed`, `timed_out`, `cancelled`, and `failed`. `--stop-scene` cancels the
scene and its active movement. `--wait` exits `0`, `4`, `5`, or `6` for
completed, timed out, cancelled, or failed respectively.

The selected scene library is recursive, including its `user/` subdirectory.
User-authored poses load from `motion/user/poses/<version>/`, and user motions
live under `motion/motions/<version>/user/`, where `version` is `v1` or `v2`.
Built-in names cannot be shadowed. At startup and
reload, `oriond` validates every pose against the active driver limits and
validates every motion keyframe reference and timing value.

The private Unix protocol supports `joint limits` so the authenticated Studio
gateway can report the running driver's calibrated ranges without exposing
servo registers. It also supports `asset reload`, which reloads poses,
motions, and scenes together and atomically replaces the validated runtime
libraries while no movement or scene is active. `scene reload` remains the
narrower scene-only operation.

The private `scene preview DOCUMENT` command is reserved for the authenticated
Studio gateway. It parses one inline v2 scene against the loaded pose, motion,
and audio libraries, then starts the normal scene
coordinator without adding it to the library or filesystem. The gateway limits
the compact document to 3,000 UTF-8 bytes so the complete command stays within
the Unix protocol's fixed 4,096-byte input boundary. Preview still uses normal
scene run IDs, status, cancellation, movement validation, and lifecycle rules.

Reload re-reads the daemon's configured scene directory, validates all pose,
motion, and audio-cue references, and atomically replaces the in-memory catalog
only when no scene is active. No command accepts an arbitrary asset path;
inline preview is the sole non-persisted scene-body operation and the raw
socket remains local to the Pi.

Unix requests are UTF-8 lines terminated by a newline (or a client write-side
EOF). The server retains partial reads and writes without blocking the motor
loop, bounds pending clients to 32, and retires incomplete connections after
one second. Oversized commands are rejected before dispatch.

For manual development, build and run `oriond` directly from this source tree
only after stopping the installed service. Normal Pi operation uses the executable
selected by `oriond.service`.

## Speech playback

The Pi's Piper Alba worker generates response audio. The onboard
coordinator buffers and uploads mono PCM16 24 kHz WAV through the local
authenticated gateway. `oriond` accepts a validated spool identifier through its private
`speech file` operation and owns speaker playback. Streaming uses `speech stream`,
ordered `speech append` commands and an explicit `speech end`, all under one run
ID and one player process. Stream and append commands accept an optional final
`cues=agree,disagree` token (at most four canonical names). For scoped voice
streams it follows the session ID; it never bypasses session ownership checks.
Each cue uses its chunk's first audio frame. One-shot `speech file` playback
has no reaction cues. See [streaming replies](../docs/voice-architecture.md#streaming-replies-and-timing)
for buffering, limits and timing semantics.

Inspect or cancel playback with:

```bash
runtime/target/release/oriond --speech-status
runtime/target/release/oriond --stop-speech
```

Speech states are `queued`, `playing`, `completed`, `failed`, and `cancelled`.
Only the active run and most recent terminal result are retained. The spool
WAV is removed after completion, cancellation, or playback failure. Stream chunks
remain bounded to two seconds. The stream accepts thirty minutes of audio and
rejects more with an explicit sanity-limit reason. PCM is released after the
player accepts it; energy frames are computed incrementally, including partial
frames across chunk boundaries. Compact, absolute-indexed animation analysis is
retained until completion. Complete-WAV uploads retain their 120-second bound.
`voice SESSION keepalive` renews an active processing, thinking or speaking lease
without changing phase, history or feedback; stale and expired IDs are rejected.

`SpeechCoordinator` validates and analyzes the waveform, while
`CharacterCoordinator` composes one anchor-relative utterance performance and
the daemon drives the `speaking_energy` light. See
[Speech performance policy](../docs/motion-reference.md#speech-performance-policy)
for the animation policy.

The Pi listener captures stereo audio (ReSpeaker HAT on V1, XVF3800 on V2) and runs Rustpotter, then
uses Silero to find speech boundaries and sends complete mono utterances to the
onboard coordinator for Qwen confirmation and transcription. See [Pi voice setup](../voice/README.md).

## Character startup and voice attention

V1 serving starts character mode by default: configure servos, enable holding
torque, move home, then enter idle after measured completion. V2 defaults to
maintenance startup with character mode off. Use `--character-on-start on|off`
to choose explicitly. On V2 with gaze available, a confirmed wake can clear
Stop's gaze latch for an already powered, enabled character; it cannot recover
disable. See [bearing gaze](#bearing-gaze-v2).
A timed-out or cancelled home
movement leaves character off; the terminal movement remains visible in status.

The Pi listener sends `voice SESSION verify` before the short wake prefix ASR
pass; recording continues while verification runs. It sends
`voice SESSION confirmed` after ASR accepts the wake and
waits for its acknowledgement. On V1, if direction is known with confidence at least
0.75, it may then send `voice SESSION attend_left AGE_MS` or `attend_right AGE_MS`.
The rest coordinator waits for home and checks that the observation is still
younger than three seconds before requesting attention.

On V2 the listener asks local vision for a speaker, then sends
`voice SESSION attend_bearing AZ EL AGE_MS`. This requires enabled gaze, finite
base-frame radians, and the current confirmed session. The same three-second
age rule includes command-queue and homing delay. Gaze compiles a neck-first
turn, and successful measured completion captures the temporary facing anchor.
Visual attention bounds base travel to 0.65 radians from measured start and
solves the remaining bearing with the neck. It rejects a final pose outside
calibrated reach. Owned background motion is stopped and feedback refreshed
before compiling the attention trajectory and checking its speed. Scene,
speech and Stop retain priority; an accepted current-session wake confirmation
can clear a powered character's Stop latch before visual attention is queued.
Disable blocks visual attention until explicit recovery.
`voice SESSION attend_scan AGE_MS` makes one short calibrated neck sweep when
the camera reports no face. Camera-off or unavailable vision sends no attention.
See [speaker selection and freshness](../docs/voice-architecture.md#direction-evidence).

The lower-level `character attend left CONFIDENCE` and `character attend right
CONFIDENCE` commands remain available for explicit attention requests. They
require confidence in [0.75, 1], a powered available character, and a bounded yaw
transition. The character holds the completed attention anchor, then returns to
the prior anchor 15 seconds after neutral inactivity. Explicit foreground work
discards that pending return. See the
[animation catalogue](../docs/motion-reference.md#animation-catalogue).

## Agent lighting commands

The authenticated gateway translates `lamp_effect` operations into the private
`lamp-effect JSON` daemon command. JSON fields are optional `brightness` (0–1),
`effect` (`solid` or a supported lighting effect), and `colors` (one or two
arrays of four integer RGBW channels, 0–255). An update must supply at least one
non-null field. Invalid patches leave the existing program unchanged.

Brightness-only updates preserve the palette and effect; zero brightness is
fully off. Lamp programs resume after higher-priority voice feedback or speech.
Scene or speech playback rejects changes until it finishes. The existing
`lamp R G B W` command still sets a steady color. `lamp-status` reads the manual
program as `lamp` with brightness in percent, effect and RGBW colors, or null when
no manual program exists. The gateway exposes this through `lamp_status` and the
agent through `get_lighting`. Starting the character clears the manual program.

## Camera viewer indicator

On V2, `camera-viewer 3` leases a steady cyan band on three adjacent ring pixels
for three seconds; `camera-viewer 0` clears it. The band sits at the top centre
when viewed from the front, on indices 15–17. Its green and blue channels stay
at 255, with red and white at zero; it never pulses or dims. The other 21 pixels
show the underlying lighting unchanged.
Other lease values and V1 requests are rejected. The authenticated gateway
manages the leases for [streams and snapshots](../orion_studio/README.md#camera-feed).
A lost gateway cannot leave a permanent marker.

The band overrides manual lighting, scenes, speech and ringing alerts only on
its three pixels while viewing. It does not change their preferences, timing,
audio, torque or motion. Going to rest, resting, waking and faults suppress the
indicator, including during sunrise; the existing rest darkness mask also
applies. The underlying RGBW frame continues updating without the indicator,
so clearing or expiry restores the latest lighting. Lease handling uses the existing queued
command socket and bounded comparisons; no camera, file or model work enters
the control tick.

## Mode and alert commands

The private Unix socket accepts these commands. The gateway exposes them through
its authenticated `routines` operation; Studio and agent tools use that route.

```text
routines status
routines {"action":"set_mode","mode":"lamp"}
routines {"action":"set_mode","mode":"idle"}
routines {"action":"set_sound","kind":"alarm","sound":"club_alarm"}
routines {"action":"set_sound","kind":"timer","sound":"funny_alarm"}
routines {"action":"timer","seconds":300,"label":"Tea"}
routines {"action":"alarm","due_unix":1893571200,"label":"Morning"}
routines {"action":"list"}
routines {"action":"cancel","id":1}
routines {"action":"stop"}
sleep CURRENT_CONFIRMED_VOICE_SESSION_ID
```

Alarm timestamps are Unix seconds and must be in the future. `sleep` waits for
the owning voice session and its speech to finish. `character rest` remains the
immediate controlled rest command. The `character status` response includes
`rest.mode`, `rest.sleep_requested` and `routines`; alert status includes pending
and recent entries, remaining ringing time and playback errors. `sounds` contains
the saved `alarm` and `timer` defaults; `available_sounds` supplies IDs and display
names, and `ring_sound` identifies the active sound. Supported IDs are `two_tone`
(the default), `club_alarm` and `funny_alarm`. `set_sound` changes only the named
preference and takes effect when a later alert group starts. See
[timer and alarm behavior](../docs/system-architecture.md#timers-and-alarms) and
[saved state](../docs/configuration.md#saved-files).

## Reminders and recurring alerts

The `routines` JSON command accepts `reminder` with `label` and exactly one of
`due_unix`, `seconds`, or `repeat`; `alarm` accepts `label` and exactly one of
`due_unix` or `repeat`. A repeat contains unique weekday names (`mon` through
`sun`) and `time: HH:MM`. The runtime discovers and saves the Pi IANA zone.
A time in a spring-forward gap rings at the first valid minute after the gap (01:30 in London rings at 02:00 local). Autumn folds use the earlier occurrence and ring once that day. Firing and
saving the next occurrence share one atomic write. Old alert files load with
new fields defaulted; older runtime releases cannot read the extended file.

Reminders enter `announcing`; `announced { id }` starts a two-minute acknowledgement
window. An unacknowledged reminder announces once more, then becomes `missed`.
`announce_failed { id }` records delivery failures. The runtime retries after 30 seconds, then two minutes, and stops after three failures: reminders become `missed`, and alarm notices or briefings are dropped. Counts and backoff survive restart. Offline delays up to twelve hours announce late. Relative reminder and retry
deadlines use elapsed time while running. `stop` dismisses only ringing alarms
and timers. Unspoken reminders remain pending for delivery after a three-second
dismissal delay; a longer retry backoff stays intact. Cancelling the last ringing
alert uses the same delay and preserves reminders. Only a dismissed ringing alarm
or timer becomes the snooze target. `dismiss_announcement { id }` acknowledges
the specific reminder or clears the alarm notice interrupted during playback.
Wake while an announcement is waiting or thinking preserves it for later delivery.
A labelled dismissed alarm queues its label for speech.
Cancellation affects one occurrence; use `cancel { id, series: true }` to cancel the recurring series as well. A snooze has its own series and never repeats a briefing or sunrise. `snooze` accepts optional `minutes` (1–60, default 9) within two minutes of dismissal;
it creates a relative countdown preserving kind and label. Storage allows 32 active
and 64 total alerts; recurring creation reserves space for its next occurrence. Missed, cancelled and dismissed reminders release their in-memory acknowledgement deadlines.

Announcement voice ownership authorizes audio while resting without confirming
user activity or enabling torque. See [voice architecture](../docs/voice-architecture.md).

Alarms accept optional `briefing: true`. Dismissal creates a persisted announcement with purpose `briefing` after a three-second delay, including for an empty label; `announced` clears that notice. The coordinator supplies its agent response through the existing announcement playback session.

`lamp-effect` accepts optional `transition_seconds` (0–3600). Brightness and RGBW interpolate from the current manual state using monotonic time; the target effect applies immediately. Another lamp command cancels the fade. Scenes and speech retain foreground ownership while fades keep advancing. Ordinary fades advance behind the rest darkness mask. Explicit `character start` cancels a light routine and clears its manual light.

Alarms accept `sunrise_minutes` (5–60). The earliest eligible alarm fades from 3% amber `[255,100,0,80]` to full warm white `[40,20,4,210]`. Sunrise bypasses only the rest light mask, without motion or user activity. Dismissal leaves full warm white on until a lamp command or character start changes it; cancellation restores the previous manual lamp. A manual change stops an active sunrise without restarting it on the next tick.

`routines {"action":"start_light_routine","routine":"wind_down","minutes":20}` fades to 8% warm white then requests the ordinary deferred sleep lifecycle: speech, scenes and confirmed voice activity finish before rest. Focus (5–120 minutes) selects full cool white `[150,190,255,110]` and creates a persisted timer labelled `Focus session`; ringing changes the lamp to warm white. The temporary lamp-mode override ends after ringing, restoring the saved mode. Starting another routine, a manual lamp command or explicit character start cancels the routine and its focus timer. `stop_light_routine` restores the previous manual lamp. Fades/routine ownership are ephemeral: restarting cancels wind-down without requesting delayed rest and drops focus light ownership while the persisted focus timer retains its deadline and sound; the saved mode is never overwritten by a routine. `start_light_routine` discloses restart cancellation in its tool description.



### Roll back to a runtime without reminders

Before the first save that adds reminder/recurrence fields, Orion preserves the exact legacy file at `~/.config/orion/routines.json.pre-reminders`. The backup is created once and never replaced; if it cannot be secured, the alert-state save fails and the legacy file stays intact. A fresh installation, or a file already written in the extended format, has no legacy snapshot to preserve.

Stop `oriond` before rollback. Keep a separate copy of the extended `routines.json`, restore `routines.json.pre-reminders` over `routines.json`, deploy the older runtime, and restart it. Alerts and changes made after the backup are absent from the restored snapshot. The sound companion file may remain; its ringing-window key prevents stale sound state from changing restored alerts. An older runtime rejects the extended file's unknown fields, so changing only the executable is insufficient.
