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
STS3215 bus. Begin a new checkout or hardware change with a torque-off state
snapshot:

```bash
runtime/target/debug/oriond --check \
  --port /dev/ttyACM0 \
  --calibration "$HOME/.config/orion/servo_calibration.json"
```

The hardware lifecycle is `--serve`, `--configure`, `--enable`, motion
commands, then `--disable`.
Neither daemon command is a physical emergency stop; an accessible hardware
torque/power interruption remains required during physical trials.

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
