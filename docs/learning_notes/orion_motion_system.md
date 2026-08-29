# How Orion moves

Orion separates motion intent from execution. A pose says where every joint
should end. A motion says which poses to visit, how long each transition takes,
and how long to hold each arrival.

```text
motion/config/poses.yaml
motion/motions/**/*.yaml
            |
            +--> Rust motion library --> oriond --> physical servos
            +--> Python trajectory library --> MuJoCo actuators
```

## Poses

Each named pose contains exactly Orion's five joints in radians. The values use
the captured physical zero as joint-space zero and must fit inside the accepted
calibration ranges.

## Motions

Each keyframe references a named pose and contains:

- `duration`: time spent moving to the pose;
- `hold`: time spent stationary after arrival.

The runtime begins a motion from the latest measured physical position, not
from an assumed previous pose. It uses quintic interpolation so each transition
starts and ends with zero velocity and acceleration.

## Physical execution

`oriond --serve` owns the STS3215 connection. Configure and enable it before
sending `--goto` or `--play`. Commands return immediately while movement
continues in the daemon, so scripts must wait for the reported duration or poll
`--status` before issuing another move.

## Simulation execution

MuJoCo loads the same YAML and uses the portable Python trajectory generator.
The simulator adds physics, stability, settling, and torque analysis without
changing the authored motion format.

## Validation

The motion tests check schema, joint order, ranges, timing, quintic dynamics,
forbidden regions, and report consistency. The Rust tests independently verify
native YAML loading, keyframe sampling, daemon behavior, and both hardware and
MuJoCo driver boundaries. Both suites must pass when a shared pose or motion
changes.
