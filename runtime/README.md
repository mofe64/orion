# Orion Rust runtime

`runtime` is Orion's ROS-independent native Rust runtime. It implements the
`oriond` command protocol, lifecycle, pose and motion loading, quintic
interpolation, calibration contract, STS3215 profile, and 50 Hz state
snapshots.

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
runtime/target/debug/oriond --goto home --duration 3.0
runtime/target/debug/oriond --play look_at_left_expressive
runtime/target/debug/oriond --stop
runtime/target/debug/oriond --disable
```

Use `--socket`, `--scene`, `--python`, or `--start-pose` to override the
defaults. The MuJoCo bridge reports measured positions and velocities and
accumulates the shared base translation, tilt, height, and contact policy in
`motion/config/stability_limits.yaml`.

## Physical hardware

The Rust transport has been validated on Orion's Raspberry Pi and five-servo
STS3215 bus. Run all commands directly from the source checkout; Orion is not
installed as a system service.

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
runtime/target/release/oriond --goto home --duration 3.0
```

Run an authored movement:

```bash
runtime/target/release/oriond --play look_at_left_expressive
```

Stop the current movement and hold its current commanded position:

```bash
runtime/target/release/oriond --stop
```

### Normal shutdown

Move Orion to its captured mechanical rest pose before disabling torque:

```bash
runtime/target/release/oriond --goto rest --duration 3.0
runtime/target/release/oriond --status
```

`--goto` starts an asynchronous trajectory. Repeat `--status` until its JSON
reports `"mode":"holding"`; do not disable while it reports
`"mode":"moving"`. Once Orion has reached rest and is holding there, disable
torque:

```bash
runtime/target/release/oriond --disable
```

Then stop Terminal 1 with `Ctrl+C`.

The normal hardware lifecycle is `--serve`, `--configure`, `--enable`, motion
commands, `--goto rest`, confirmed holding, and finally `--disable`.
Neither `--disable` nor stopping the daemon is a physical emergency stop; an
accessible hardware torque/power interruption remains required during physical
trials.

## Port structure

- `src/transport.rs` — raw `rustypot` STS3215 serial and packet boundary.
- `src/driver.rs` — calibration conversions and servo safety sequence.
- `src/daemon.rs` — backend-independent lifecycle and command state machine.
- `src/socket.rs` — local Unix command server/client.
- `src/pose.rs`, `motion.rs`, `trajectory.rs` — shared motion semantics.
- `src/mujoco.rs` and `mujoco_bridge.py` — native simulation backend.
- `src/main.rs` — `oriond` arguments and 50 Hz control loop.

During development, build and run `oriond` directly from this source tree. It
is not installed as a system service.
