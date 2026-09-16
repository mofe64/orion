# Orion Rust runtime

`oriond` owns Orion's movement, RGBW light and speaker playback. It loads poses
and motions, compiles smooth joint trajectories, applies calibration, checks
measured completion and coordinates character behavior, scenes and speech
animation. The hardware and MuJoCo backends use the same 50 Hz runtime loop.

See the [system architecture](../docs/system-architecture.md) for
workstation/Pi boundaries and device ownership.

For movement internals, use:

- [Motion and animation architecture](../docs/motion-and-animation-architecture.md)
- [Character animation design](../docs/character-animation.md)
- [Trajectory and joint-control reference](../docs/trajectory-and-joint-control.md)
- [Motion asset reference](../docs/motion-assets.md)

The physical transport uses
[`rustypot`](https://github.com/pollen-robotics/rustypot) for the STS3215
serial packet format, synchronized reads/writes, and communication. This
servo-wire protocol is unrelated to Orion's asset format. Orion retains its
own raw register map and conversions for firmware bytes at addresses 0/1, a
one-byte maximum-acceleration register at address 85, and project-specific
encoder/velocity conversions.

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

## Deploy an update to the Raspberry Pi

Use the [Pi deployment procedure](../docs/quickstart.md#deploy-to-the-pi) from the
workstation after committing and pushing the intended code. It prepares all four
services in a separate release, then switches paths after confirmed mechanical
rest. The [configuration reference](../docs/configuration.md#raspberry-pi-deployment)
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
See [the speech runtime walkthrough](../runtime.md) for planning and execution.

## Lighting, audio, and local scenes

The physical light adapter targets Orion's 40-pixel Adafruit RGBW shield on
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

The physical audio adapter uses the stable Advanced Linux Sound Architecture
(ALSA) pulse-code modulation (PCM) device
`plughw:CARD=seeed2micvoicec,DEV=0`. It applies the confirmed ReSpeaker V2 JST
mixer route whenever the hardware daemon starts. Named, local, stereo WAV
cues live under `audio/cues/` and can be checked without starting the
servo daemon:

```bash
runtime/target/release/oriond --play-cue acknowledge_warm
```

The direct command blocks until `aplay` exits and returns nonzero if playback
fails. Do not run it concurrently with a hardware daemon that may also own the
ALSA PCM.

Direct warm-cue playback and the complete `acknowledge_left` and
`acknowledge_right` motion/light/audio scenes use the same physical ReSpeaker
path as the character coordinator.

Portable scenes live under `scenes/`. The v2 format coordinates
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
initial state, configures the ReSpeaker mixer, and owns both devices until the
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

The scene library is recursive, including `scenes/user/`. User-authored poses
are loaded recursively from `motion/user/poses/`, and user motions live under
`motion/motions/user/`. Built-in names cannot be shadowed. At startup and
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

The Pi's Pocket worker generates response audio. The onboard coordinator buffers
and uploads mono PCM16 24 kHz WAV through the local authenticated gateway. `oriond` accepts a validated spool identifier through its private
`speech file` operation and owns ReSpeaker playback. Streaming uses `speech stream`,
ordered `speech append` commands and an explicit `speech end`, all under one run
ID and one player process. See [streaming replies](../docs/voice-architecture.md#streaming-replies-and-timing)
for buffering, limits and timing semantics.

Inspect or cancel playback with:

```bash
runtime/target/release/oriond --speech-status
runtime/target/release/oriond --stop-speech
```

Speech states are `queued`, `playing`, `completed`, `failed`, and `cancelled`.
Only the active run and most recent terminal result are retained. The spool
WAV is removed after completion, cancellation, or playback failure.

`SpeechCoordinator` validates and analyzes the waveform, while
`CharacterCoordinator` composes one anchor-relative utterance performance and
the daemon drives the `speaking_energy` light. See
[Character animation design](../docs/character-animation.md#speech-driven-animation)
for the animation policy.

The Pi listener captures stereo ReSpeaker audio and runs Rustpotter, then
uses Silero to find speech boundaries and sends complete mono utterances to the
onboard coordinator for Qwen confirmation and transcription. See [Pi voice setup](../voice/README.md).

## Character startup and voice attention

Serving starts character mode by default: configure servos, enable holding torque,
move home, then enter idle after measured completion. Use
`--serve --character-on-start off` for an observation-only maintenance startup.
Studio Stop lasts until an explicit character start or the next daemon startup
with character mode enabled. A timed-out or cancelled home
movement leaves character off; the terminal movement remains visible in status.

The Pi listener sends `voice SESSION verify` before the short wake prefix ASR
pass; recording continues while verification runs. It sends
`voice SESSION confirmed` after ASR accepts the wake and
waits for its acknowledgement. If direction is known with confidence at least
0.75, it may then send `voice SESSION attend_left AGE_MS` or `attend_right AGE_MS`.
The rest coordinator waits for home and checks that the observation is still
younger than three seconds before requesting attention.

The lower-level `character attend left CONFIDENCE` and `character attend right
CONFIDENCE` commands remain available for explicit attention requests. They
require confidence in [0.75, 1], a powered available character, and a bounded yaw
transition. The character holds the completed attention anchor, then returns to
the prior anchor 15 seconds after neutral inactivity. Explicit foreground work
discards that pending return. See the
[animation catalogue](../docs/orion-animation-catalogue.md#motion-review).

## Agent lighting commands

The authenticated gateway translates `lamp_effect` operations into the private
`lamp-effect JSON` daemon command. JSON fields are optional `brightness` (0–1),
`effect` (`solid` or a supported lighting effect), and `colors` (one or two
arrays of four integer RGBW channels, 0–255). An update must supply at least one
non-null field. Invalid patches leave the existing program unchanged.

Brightness-only updates preserve the palette and effect; zero brightness is
fully off. Lamp programs resume after higher-priority voice feedback or speech.
Scene or speech playback rejects changes until it finishes. The existing
`lamp R G B W` command still sets a steady color.

## Mode and alert commands

The private Unix socket accepts these commands. The gateway exposes them through
its authenticated `routines` operation; Studio and agent tools use that route.

```text
routines status
routines {"action":"set_mode","mode":"lamp"}
routines {"action":"set_mode","mode":"idle"}
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
and recent entries, remaining ringing time and playback errors. See
[timer and alarm behavior](../docs/system-architecture.md#timers-and-alarms) and
[saved state](../docs/configuration.md#saved-files).
