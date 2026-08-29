# Orion Rust runtime

`runtime_rust` is the pure-Rust port of Orion's ROS-independent C++ runtime.
It preserves the current `oriond` command protocol, lifecycle, pose and motion
loading, quintic interpolation, calibration contract, STS3215 profile, and
50 Hz state snapshots.

The physical transport uses
[`rustypot`](https://github.com/pollen-robotics/rustypot) for protocol-v1 packet
parsing, synchronized reads/writes, and serial communication. Orion retains its
own raw register map and conversions because its proven C++ contract includes
firmware bytes at addresses 0/1, a one-byte maximum-acceleration register at
address 85, and project-specific encoder/velocity conversions.

## Build and test

Install a current Rust toolchain, then run from the repository root:

```bash
cargo build --manifest-path runtime_rust/Cargo.toml
cargo test --manifest-path runtime_rust/Cargo.toml --all-targets
```

The tests cover the C++ parity contract and launch Orion's existing native
MuJoCo model through the same Rust daemon state machine used by hardware.
MuJoCo tests expect the repository Python environment at `.venv/bin/python`.

## MuJoCo-first daemon

Run the daemon without opening a serial port:

```bash
runtime_rust/target/debug/oriond --serve --backend mujoco \
  --start-pose attentive
```

In another terminal, use the normal client commands:

```bash
runtime_rust/target/debug/oriond --status
runtime_rust/target/debug/oriond --configure
runtime_rust/target/debug/oriond --enable
runtime_rust/target/debug/oriond --goto home --duration 3.0
runtime_rust/target/debug/oriond --play look_at_left_expressive
runtime_rust/target/debug/oriond --stop
runtime_rust/target/debug/oriond --disable
```

Use `--socket`, `--scene`, `--python`, or `--start-pose` to override the
defaults. The MuJoCo bridge reports measured positions and velocities and
accumulates the shared base translation, tilt, height, and contact policy in
`motion/config/stability_limits.yaml`.

## Physical hardware gate

Do not test the Rust transport on Orion until the Rust suite, the C++ suite,
and the existing repository MuJoCo suite all pass. Then begin torque-off:

```bash
runtime_rust/target/debug/oriond --check \
  --port /dev/ttyACM0 \
  --calibration "$HOME/.config/orion/servo_calibration.json"
```

Only after comparing the Rust telemetry to the known C++ output should the
daemon be started against hardware. The existing lifecycle remains:
`--serve`, `--configure`, `--enable`, motion commands, then `--disable`.
Neither daemon command is a physical emergency stop; an accessible hardware
torque/power interruption remains required during physical trials.

## Port structure

- `src/transport.rs` — raw `rustypot` STS3215 serial and packet boundary.
- `src/driver.rs` — calibration conversions and servo safety sequence.
- `src/daemon.rs` — backend-independent lifecycle and command state machine.
- `src/socket.rs` — local Unix command server/client.
- `src/pose.rs`, `motion.rs`, `trajectory.rs` — shared motion semantics.
- `src/mujoco.rs` and `mujoco_bridge.py` — native simulation backend.
- `src/main.rs` — `oriond` arguments and 50 Hz service loop.

The C++ runtime remains in `runtime/` as the comparison oracle until physical
feature-parity trials are complete.
