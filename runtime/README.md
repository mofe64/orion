# Orion Rust runtime

`runtime` is Orion's ROS-independent native Rust runtime. It implements the
`oriond` command protocol, lifecycle, pose and motion loading, whole-action
piecewise-quintic trajectory compilation, calibration contract, STS3215
profile, 50 Hz state snapshots, the Pi 5 red-green-blue-white (RGBW) output
backend, ReSpeaker V2 WAV playback, character coordination, and multimodal
scenes.

See the [system architecture](../docs/system-architecture.md) for
workstation/Pi boundaries and device ownership.

For movement internals, use the canonical cross-system documents:

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

Use a toolchain that supports the crate's Rust 2024 syntax; validation uses
Rust 1.98.0. Run from the repository root:

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

During source-run development, Git is Orion's deployment package. Commit and
push the intended `main` revision before deploying from the development
workstation.

The workstation needs pnpm and Node.js 20.19 or later in the 20.x line,
or Node.js 22.12 or later, for Studio's deployment preflight. Validation uses
Node.js 24.19.0. Run from the repository root:

```bash
scripts/deploy_pi.sh
```

The command connects to `mofe@orion.local` over SSH, uploads the deployment
bootstrap to a temporary file and runs it in an SSH terminal. This leaves
terminal input available for sudo authentication; the temporary file is
removed when the remote command exits. Override the target when needed with `--host`, `--root`, or
`--branch`. SSH host identity and key access must already be trusted normally;
the script never disables host-key checking. Enter the Pi user's sudo password
in the terminal when prompted. The account must be permitted to install
packages and manage services; passwordless sudo is needed only for unattended
runs. Deployment does not change sudoers or request the password through chat.

The remote phase prepares the complete runtime, gateway, listener, and onboard
voice/agent release from the selected commit. It leaves the Pi checkout and motion
catalog intact. Builds and tests finish before the service switch; the runtime
must confirm mechanical rest and torque-off before it is stopped. The installer
preserves installed arguments and overrides, checks all four services and their
readiness, and restores the previous installation on activation failure. See the
[Pi quickstart](../docs/quickstart.md#deploy-to-the-pi) for prerequisites, settings
preservation, and rollback.

The simulator-only MuJoCo integration test remains a workstation pre-push gate.
Deployment compiles a trajectory against the existing catalog without executing
it. It does not run physical expression smoke tests. Normal runtime startup may
move Orion home. Use `--character-on-start off` for maintenance that must remain
torque-off; in that mode, an explicit movement request can prepare and enable the
servos. Use **Release torque** only after mechanical rest is confirmed.

Character startup arms [automatic rest and confirmed waking](../docs/system-architecture.md#automatic-rest-and-waking).
`--rest-after-seconds` configures the inactivity deadline. Studio's **Go to rest**
also follows measured rest completion, fades the light off, and releases torque.
Character Stop and low-level `goto rest` retain their separate contracts.
Native MuJoCo's captured-rest contact mismatch is covered by the
[rest/wake integration tests](tests/test_rest.py); a failed rest run keeps torque enabled.

Logs are owned by journald:

```bash
sudo systemctl status oriond.service orion-studio-gateway.service orion-listener.service
journalctl -u oriond.service -u orion-studio-gateway.service -u orion-listener.service
```

Calibration and the Studio pairing token remain under `~/.config/orion/`.
The full-stack installer merges installed service configuration; templates under
`scripts/systemd/` supply defaults only for missing units. Runtime and gateway
executables come from the release, while their working directory remains the
existing catalog root so local poses and scenes survive updates.

## MuJoCo-first daemon

Run the daemon without opening a serial port:

```bash
runtime/target/debug/oriond --serve --backend mujoco \
  --start-pose attentive
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
STS3215 bus. The installed systemd service still executes the release binary
and assets directly from the source checkout; no second runtime copy exists.
Stop `oriond.service` before opening the serial, RGBW, or audio devices with a
manual commissioning process.

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
`last_motion`; there is no movement database or durable history. Planned agent
integrations must submit semantic motion names, retain the returned `run_id`,
and follow that ID through these fields. IDs reset when the daemon restarts.

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
path has been commissioned on the physical robot, including all four channels,
the full matrix, and all-off output.

The physical audio adapter uses the stable Advanced Linux Sound Architecture
(ALSA) pulse-code modulation (PCM) device
`plughw:CARD=seeed2micvoicec,DEV=0`. It applies the confirmed ReSpeaker V2 JST
mixer route whenever the hardware daemon starts. Named, local, stereo WAV
cues live under `audio/cues/` and can be commissioned without starting the
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
process exits. Direct lighting and cue commissioning commands therefore should
not run concurrently with the daemon. MuJoCo uses recording lighting and audio
backends with the identical scene clock and lifecycle.

Run the lighting-only scene without enabling torque:

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
reset when the source-run daemon restarts. Scene states are `executing`,
`completed`, `timed_out`, `cancelled`, and `failed`. `--stop-scene` cancels the
scene and its active movement. `--wait` exits `0`, `4`, `5`, or `6` for
completed, timed out, cancelled, or failed respectively.

The scene library is recursive, including `scenes/user/`. User-authored poses
are loaded recursively from `motion/user/poses/`, and user motions live under
`motion/motions/user/`. Built-in names cannot be shadowed. At startup and
reload, `oriond` validates every pose against the active driver limits and
validates every motion keyframe reference and timing value.

The private Unix protocol supports `joint limits` so the authenticated Studio
gateway can report the running driver's commissioned radians without exposing
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
socket remains Pi-local.

Unix requests are UTF-8 lines terminated by a newline (or a client write-side
EOF). The server retains partial reads and writes without blocking the motor
loop, bounds pending clients to 32, and retires incomplete connections after
one second. Oversized commands are rejected before dispatch.

For manual development, build and run `oriond` directly from this source tree
only after stopping the installed service. Normal Pi operation uses the
source-backed `oriond.service`.

## Speech playback

Studio generates expressive speech with Chatterbox and uploads mono PCM16
24 kHz WAV through the authenticated gateway. The Pi does not synthesize
speech. `oriond` accepts a validated spool identifier through its private
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
forwards endpointed mono utterances to Studio for Qwen confirmation and
processing. See [Pi voice setup](../voice/README.md).

## Character startup and voice attention

Serving starts character mode by default: configure servos, enable holding torque,
move home, then enter idle after measured completion. Use
`--serve --character-on-start off` for an observation-only maintenance startup.
Studio Stop lasts until an explicit character start or the next daemon startup
with character mode enabled. A timed-out or cancelled home
movement leaves character off; the terminal movement remains visible in status.

The Pi listener sends `voice SESSION verify` before the short wake-prefix ASR
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
