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

The next runtime increment is explicit servo-profile application followed by a
snap-free torque lifecycle. Motion commands will only be added after those
operations are testable with the fake transport.
