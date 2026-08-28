# Orion native runtime

`runtime` is Orion's ROS-independent C++ control path. It builds the existing
STS3215 driver, calibration loader, Feetech transport, and the `oriond`
executable with ordinary CMake.

The first implemented daemon operation is deliberately narrow:

```bash
oriond --check
```

It validates the calibration and five-servo bus, reads one synchronized state
snapshot, prints it, and exits. It does not enable torque, write a goal
position, apply the servo profile, or write servo registers.

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

The next runtime increment is a persistent fixed-rate state loop with a local
structured API. Torque activation and motion commands will be added after that
loop is testable with the fake transport.
