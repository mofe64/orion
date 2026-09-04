# Orion Rust runtime

`runtime` is Orion's ROS-independent native Rust runtime. It implements the
`oriond` command protocol, lifecycle, pose and motion loading, whole-action
piecewise-quintic trajectory compilation, calibration contract, STS3215
profile, 50 Hz state snapshots, the Pi 5 red-green-blue-white (RGBW) output
backend, ReSpeaker V2 WAV playback, character coordination, and multimodal
scenes.

See the [system architecture](../docs/explanation/system-architecture.md) for
workstation/Pi boundaries and device ownership.

For movement internals, use the canonical cross-system documents:

- [Motion and animation architecture](../docs/explanation/motion-and-animation-architecture.md)
- [Character animation design](../docs/explanation/character-animation.md)
- [Trajectory and joint-control reference](../docs/reference/trajectory-and-joint-control.md)
- [Motion asset reference](../docs/reference/motion-assets.md)

The physical transport uses
[`rustypot`](https://github.com/pollen-robotics/rustypot) for the STS3215
serial packet format, synchronized reads/writes, and communication. This
servo-wire protocol is unrelated to Orion's asset format. Orion retains its
own raw register map and conversions for firmware bytes at addresses 0/1, a
one-byte maximum-acceleration register at address 85, and project-specific
encoder/velocity conversions.

## Build and test

Install a stable toolchain with Rust 2024 edition support (Rust 1.85 or newer),
then run from the repository root:

```bash
cargo build --manifest-path runtime/Cargo.toml
cargo test --manifest-path runtime/Cargo.toml --all-targets
```

The tests cover the complete runtime contract and launch Orion's native MuJoCo
model through the same Rust daemon state machine used by hardware.
MuJoCo tests expect the repository Python environment at `.venv/bin/python`.

## Deploy an update to the Raspberry Pi

During source-run development, Git is Orion's deployment package. Commit and
push the intended `main` revision, then run this from the development
workstation:

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

On the Pi, deployment requires the selected branch to already be checked out,
then fetches and fast-forwards it. It never switches branches, stashes, resets,
or runs a broad cleanup of the Pi checkout. The specific generated lockfile
migration is described in [Pi voice setup](../voice/README.md). Deployment stops the gateway, returns the running
Orion to `rest`, disables torque, runs gateway tests and the
Pi-compatible Rust suite, and release-builds `oriond` while the old daemon
remains safely torque-off. The simulator-only MuJoCo integration test remains a
workstation pre-push gate.

The deployment installs the locked Pi Rustpotter environment, archives the
checkout's retired voice stack, and installs/enables `oriond.service`,
`orion-studio-gateway.service` and `orion-listener.service`. It starts the rebuilt runtime, verifies its
embedded `build_revision`, configures and enables Orion, moves to
`zero_reference`, and runs the no-motion `deployment_smoke` RGBW/audio scene.
A successful trial returns to `rest`, fades lights off, disables torque, and
starts the authenticated gateway on port 7447. Any post-start failure attempts
the same resting shutdown.

All three services start on reboot. `oriond` starts the powered character by default
and moves home before scheduling idle. Studio can stop character mode for the
current daemon session. Use `--character-on-start off` for maintenance that
must remain torque-off; in that mode, an explicit movement request can prepare
and enable the servos. Use **Release torque** only after mechanical rest is
confirmed.

Logs are owned by journald:

```bash
sudo systemctl status oriond.service orion-studio-gateway.service orion-listener.service
journalctl -u oriond.service -u orion-studio-gateway.service -u orion-listener.service
```

Calibration and the Studio pairing token remain under `~/.config/orion/` and
are never replaced during ordinary updates. See [Pi voice setup](../voice/README.md)
for installation, rollback and capture checks. The unit templates are under
`scripts/systemd/`; `scripts/install_pi_services.sh` renders the configured Pi
user, home, and source-checkout paths into `/etc/systemd/system/`.

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

## Port structure

- `src/lighting.rs` — RGBW frames and the lighting-device boundary.
- `src/audio.rs` — named local cues and the audio-device boundary.
- `src/scene.rs` — versioned scene loading, validation, and monotonic playback.
- `src/transport.rs` — raw `rustypot` STS3215 serial and packet boundary.
- `src/driver.rs` — calibration conversions and servo safety sequence.
- `src/daemon.rs` — backend-independent lifecycle and command state machine.
- `src/socket.rs` — local Unix command server/client.
- `src/pose.rs`, `motion.rs`, `trajectory.rs` — shared motion semantics.
- `src/mujoco.rs` and `mujoco_bridge.py` — native simulation backend.
- `src/main.rs` — `oriond` arguments and 50 Hz control loop.

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

For manual development, build and run `oriond` directly from this source tree
only after stopping the installed service. Normal Pi operation uses the
source-backed `oriond.service`.

## Generated speech

The optional persistent Piper worker under `voice/` generates speech with
Orion's selected Ryan Medium voice without loading its ONNX model in Orion's
50 Hz Rust control loop. Follow `voice/README.md` to create its Python 3.11
environment, install the selected local models, and start
`/tmp/orion-tts.sock`.

With the worker and hardware daemon running, submit dynamic speech through the
daemon-owned ReSpeaker playback backend:

```bash
runtime/target/release/oriond --speak "Hello. I am Orion." --wait
runtime/target/release/oriond --speech-status
runtime/target/release/oriond --stop-speech
```

Speech states are `synthesizing`, `playing`, `completed`, `failed`, and
`cancelled`. Only the active run and most recent terminal result are retained.
The generated WAV is temporary and removed after playback. A failed `--wait`
returns exit code `7`; cancellation returns `5`.

Authenticated Studio Voice WAV uploads enter the same `SpeechCoordinator`.
The coordinator validates and analyzes the waveform, while
`CharacterCoordinator` composes one anchor-relative utterance performance and
the daemon drives the `speaking_energy` light. Both Studio Chatterbox and local
Piper therefore share movement, lighting, cancellation, anchor restoration,
run status, and temporary-file cleanup. See
[Character animation design](../docs/explanation/character-animation.md#speech-driven-animation)
for the exact animation policy.

The primary Pi listener captures stereo ReSpeaker audio and runs Rustpotter,
then forwards endpointed mono utterances to Studio for Qwen confirmation and
processing. The optional legacy offline tools remain separate. See
[Pi voice setup](../voice/README.md).

## Character startup and voice attention

Serving starts character mode by default: configure servos, enable holding torque,
move home, then enter idle after measured completion. Use
`--serve --character-on-start off` for an observation-only maintenance startup.
Studio Stop lasts until the next daemon restart. A timed-out or cancelled home
movement leaves character off; the terminal movement remains visible in status.

The Pi listener may send `character attend left CONFIDENCE` or
`character attend right CONFIDENCE` on the local socket after Qwen confirmation.
The coordinator requires confidence in [0.75, 1], a powered available character
and a bounded yaw transition. It holds the completed attention anchor, then
returns to the prior anchor 15 seconds after neutral inactivity. Explicit
foreground work discards that pending return. See the
[attention brief](../docs/explanation/voice-attention.md).
