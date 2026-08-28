# Orion native runtime

`runtime` is Orion's ROS-independent C++ control path. It builds the existing
STS3215 driver, calibration loader, Feetech transport, and the `oriond`
executable with ordinary CMake.

The direct hardware check is deliberately narrow:

```bash
oriond --check
```

It validates the calibration and five-servo bus, reads one synchronized state
snapshot, prints it, and exits. It does not enable torque, write a goal
position, apply the servo profile, or write servo registers.

The persistent observe mode samples the same state at 50 Hz and exposes the
latest versioned JSON snapshot on a local Unix socket. It remains torque-off
and does not write servo registers:

```bash
runtime/build/oriond --serve
```

From another terminal, query that daemon without opening the servo port:

```bash
runtime/build/oriond --status
```

The daemon lifecycle is explicit. Apply and verify the LeLamp-compatible servo
profile while torque remains off:

```bash
runtime/build/oriond --configure
```

Enable holding torque only after configuration. The driver first writes each
servo's measured position back as its goal, verifies those goals, and then
enables torque:

```bash
runtime/build/oriond --enable
```

No pose or trajectory is sent in holding mode. Disable torque through the same
daemon-owned connection:

```bash
runtime/build/oriond --disable
```

Move to any complete pose in Orion's existing pose library with a quintic
trajectory. The daemon validates all five targets against the physical
calibration and sends one synchronized position update per 50 Hz cycle:

```bash
runtime/build/oriond --goto rest --duration 2.0
runtime/build/oriond --goto home --duration 4.0
```

While moving, `--status` reports `mode: "moving"`, the active pose name, and
normalized progress. At completion it returns to `mode: "holding"`. Calling
`--disable` during a move cancels the trajectory and turns torque off.

The native trajectory path does not use the provisional simulation-only
velocity, acceleration, or jerk ceilings from `orion_motion`. The requested
duration determines the motion rate. Physical calibrated joint-position bounds
remain mandatory.

## Native build

On Debian or Ubuntu, the required system packages are:

```bash
sudo apt install build-essential cmake libyaml-cpp-dev libgtest-dev
```

Configure, build, and test without sourcing ROS:

```bash
cmake -S runtime -B runtime/build -DCMAKE_BUILD_TYPE=Release
cmake --build runtime/build --parallel
ctest --test-dir runtime/build --output-on-failure
```

Display the current physical state:

```bash
runtime/build/oriond --check \
  --port /dev/ttyACM0 \
  --calibration "$HOME/.config/orion/servo_calibration.json"
```

## Current boundary

The reusable driver sources temporarily remain in `ros2_ws/src/orion_hardware`
so the new native build and the existing ROS adapter share one implementation.
The ROS adapter is optional and is not part of the native build. Once the
standalone daemon lifecycle is established, the reusable sources can move to a
neutral package without changing their behavior.

The next runtime increment is the matching MuJoCo backend and richer animation
sequencing. The physical daemon already supports named-pose movement.
