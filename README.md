# Orion

Orion is an expressive robotic-lamp project with a ROS-independent C++
hardware runtime and a native MuJoCo simulation backend.

## Repository layout

- `runtime/` — C++ daemon, STS3215 driver, Feetech transport, and native tests.
- `motion/` — shared poses, motions, limits, trajectory tools, and tests.
- `description/` — neutral URDF and the shared mesh library.
- `simulation/mujoco/` — MuJoCo model, playback tools, and simulator tests.
- `hardware/servo_setup/` — one-time servo setup, calibration, and rest capture.

See [the control architecture](docs/orion_control_architecture.md) for how these
parts connect. Start physical bring-up with the
[STS3215 setup guide](hardware/servo_setup/README.md).

## Native build and test

```bash
cmake -S runtime -B runtime/build -DCMAKE_BUILD_TYPE=Release
cmake --build runtime/build --parallel
ctest --test-dir runtime/build --output-on-failure

PYTHONPATH=motion python3 -m pytest -q motion/test
```

## Demo motion sequence

With `oriond --serve` running in another terminal:

```bash
runtime/build/oriond --configure
runtime/build/oriond --enable

runtime/build/oriond --goto zero_reference --duration 5.0
sleep 6

runtime/build/oriond --goto attentive --duration 3.0
sleep 4

runtime/build/oriond --goto home --duration 3.0
sleep 4

runtime/build/oriond --play look_at_left_expressive
sleep 8

runtime/build/oriond --goto rest --duration 5.0
sleep 6
runtime/build/oriond --disable
```
