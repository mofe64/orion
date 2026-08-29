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

If torque-off settling leaves a joint just outside the inset command range, a
move begins at the nearest safe command boundary and continues inward to the
validated pose. This changes only the trajectory's starting command; it does
not widen calibration limits or alter the requested pose.

While moving, `--status` reports `mode: "moving"`, the active pose name, and
normalized progress. At completion it returns to `mode: "holding"`. Calling
`--disable` during a move cancels the trajectory and turns torque off.

Play an authored motion from the nested functional/expressive YAML library:

```bash
runtime/build/oriond --play look_at_left_expressive
runtime/build/oriond --play look_at_right_expressive
```

The daemon validates every referenced pose against the physical calibration
before starting. It then executes each quintic transition and hold without
blocking the 50 Hz state loop. During playback, `--status` includes the motion
name, current keyframe name, zero-based keyframe index, keyframe count, and
overall progress. Stop at the latest commanded position while retaining
holding torque with:

```bash
runtime/build/oriond --stop
```

`--disable` remains the command that cancels movement and turns torque off.
Unknown motions, malformed commands, and invalid targets return JSON errors
without terminating the daemon.

The native trajectory path does not use the provisional simulation-only
velocity, acceleration, or jerk ceilings from `motion/config/motion_limits.yaml`. The requested
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

## Runtime boundary

The reusable STS3215 driver and Feetech SDK live inside `runtime`; the build has
no ROS dependency. Shared pose and motion definitions live in the root
`motion` directory so the physical daemon and MuJoCo consume the same assets.

The feature-parity Rust port and its matching MuJoCo backend now live in
`runtime_rust/`. This C++ implementation remains the comparison oracle until
the Rust transport completes physical hardware trials.
