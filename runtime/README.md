# Orion Rust runtime

`runtime` is Orion's ROS-independent native Rust runtime. It implements the
`oriond` command protocol, lifecycle, pose and motion loading, quintic
interpolation, calibration contract, STS3215 profile, 50 Hz state snapshots,
the Pi 5 RGBW output backend, ReSpeaker V2 WAV playback, and multimodal scene
coordination.

The physical transport uses
[`rustypot`](https://github.com/pollen-robotics/rustypot) for protocol-v1 packet
parsing, synchronized reads/writes, and serial communication. Orion retains its
own raw register map and conversions for firmware bytes at addresses 0/1, a
one-byte maximum-acceleration register at address 85, and project-specific
encoder/velocity conversions.

## Build and test

Install a current Rust toolchain, then run from the repository root:

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

The command connects to `mofe@orion.local` over SSH and streams the remote
deployment logic. Override the target when needed with `--host`, `--root`, or
`--branch`. SSH host identity and key access must already be trusted normally;
the script never disables host-key checking. The Pi user needs passwordless
`sudo` for unattended installation and control of the two system services.

On the Pi, deployment requires the selected branch to already be checked out,
then fetches and fast-forwards it. It never switches branches, stashes, resets,
or cleans the Pi checkout. Deployment stops the gateway, returns the currently
running Orion to `rest`, disables torque, runs gateway tests and the
Pi-compatible Rust suite, and release-builds `oriond` while the old daemon
remains safely torque-off. The simulator-only MuJoCo integration test remains a
workstation pre-push gate.

The deployment then installs and enables `oriond.service` and
`orion-studio-gateway.service`, starts the rebuilt runtime, verifies its
embedded `build_revision`, configures and enables Orion, moves to
`zero_reference`, and runs the no-motion `deployment_smoke` RGBW/audio scene.
A successful trial returns to `rest`, fades lights off, disables torque, and
starts the authenticated gateway on port 7447. Any post-start failure attempts
the same resting shutdown.

Both services start on reboot. `oriond` deliberately boots in torque-off
observe mode. The authenticated gateway remains reachable, and Studio's first
explicit movement Run uses the semantic `prepare_movement` operation to
configure and enable torque. Lighting/audio-only scenes do not energize the
servos. Use **Release torque** in Studio when holding is no longer needed.

Logs are owned by journald:

```bash
sudo systemctl status oriond.service orion-studio-gateway.service
journalctl -u oriond.service -u orion-studio-gateway.service
```

Calibration and the Studio pairing token remain under `~/.config/orion/` and
are never replaced during ordinary updates. The unit templates are under
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
within tolerance for the full settle duration. The current defaults are
`0.05 rad`, `0.05 rad/s`, `0.25 s` settled, and a `2.0 s` settling timeout.

Status JSON keeps only the active `motion` and the most recent terminal
`last_motion`; there is no movement database or durable history. A future agent
should submit semantic motion names, retain the returned `run_id`, and follow
that ID through these fields. IDs reset when the daemon restarts.

`--wait` is a thin client over the same status contract. It exits `0` for
`completed`, `4` for `timed_out`, and `5` for `cancelled`. Daemon command
rejection exits `3`, invalid CLI usage exits `2`, and transport/runtime errors
exit `1`.

Stop the current movement and hold its current commanded position:

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
Pi 5 BCM12. After installing and reboot-verifying the persistent RP1 PWM setup
described in `hardware/lighting/README.md`, direct output is available without
starting the servo daemon:

```bash
runtime/target/release/oriond --light 8 3 0 20
runtime/target/release/oriond --light-pixel 0 0 0 0 8
runtime/target/release/oriond --lights-off
```

Arguments are logical `RED GREEN BLUE WHITE` bytes from 0 through 255. The
adapter performs the physical GRBW ordering and 800 kHz symbol encoding. This
path has been commissioned on the physical robot, including all four channels,
the full matrix, and all-off output.

The physical audio adapter uses the stable ALSA PCM
`plughw:CARD=seeed2micvoicec,DEV=0`. It applies the confirmed ReSpeaker V2 JST
mixer route whenever the hardware daemon starts. Named, local, stereo WAV
cues live under `audio/cues/` and can be commissioned without starting the
servo daemon:

```bash
runtime/target/release/oriond --play-cue acknowledge
```

The direct command blocks until `aplay` exits and returns nonzero if playback
fails. Do not run it concurrently with a hardware daemon that may also own the
ALSA PCM.

Direct `acknowledge` playback and the complete `acknowledge_left` and
`acknowledge_right` motion/light/audio scenes have been commissioned on the
assembled robot through the source-run release daemon.

Portable scenes live under `scenes/`. Version 1 can play an existing motion,
go to an existing pose, fade to a uniform 8-bit RGBW value, and dispatch a
named audio cue. Scene files are validated against the pose, motion, and cue
libraries before playback. All events use seconds from one supplied monotonic
start time.

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
runtime/target/release/oriond --run-scene lighting_acknowledge --wait
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

Reload re-reads the daemon's configured scene directory, validates all pose,
motion, and audio-cue references, and atomically replaces the in-memory catalog
only when no scene is active. It does not accept a path or scene body over the
local command socket.

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

The separate local wake worker captures transient microphone PCM and publishes
ordered `HELLO WORLD` and transcribed-command events through
`/tmp/orion-wake.sock` for the future agent
runtime. It does not yet perform speech-to-text or intent routing. See
`voice/README.md` for the physical test sequence.
