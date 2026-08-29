# Orion

Orion is an expressive robotic-lamp project with a ROS-independent Rust runtime
and a native MuJoCo simulation backend.

## Repository layout

- `runtime/` — native Rust runtime using `rustypot`, with hardware and
  MuJoCo backends behind the same daemon state machine.
- `motion/` — shared poses, motions, limits, trajectory tools, and tests.
- `description/` — neutral URDF and the shared mesh library.
- `simulation/mujoco/` — MuJoCo model, playback tools, and simulator tests.
- `hardware/servo_setup/` — one-time servo setup, calibration, and rest capture.

See [the control architecture](docs/orion_control_architecture.md) for how these
parts connect. Start physical bring-up with the
[STS3215 setup guide](hardware/servo_setup/README.md).

## Native build and test

```bash
cargo build --manifest-path runtime/Cargo.toml --release --locked
cargo test --manifest-path runtime/Cargo.toml --all-targets

PYTHONPATH=motion python3 -m pytest -q motion/test
```

See [`runtime/README.md`](runtime/README.md) for the complete Rust build,
simulation, and physical-hardware workflow.

## Demo motion sequence

With the source-tree `oriond --serve` running in another terminal:

```bash
ORIOND=runtime/target/release/oriond

$ORIOND --configure
$ORIOND --enable

$ORIOND --goto zero_reference --duration 5.0
sleep 6

$ORIOND --goto attentive --duration 3.0
sleep 4

$ORIOND --goto home --duration 3.0
sleep 4

$ORIOND --play look_at_left_expressive
sleep 8

$ORIOND --goto rest --duration 5.0
sleep 6
$ORIOND --disable
```
