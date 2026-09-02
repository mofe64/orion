# How Orion moves

Motion assets and limits live in [`motion/`](../../motion/). Runtime lifecycle
and execution behaviour live in [`runtime/`](../../runtime/).

Orion separates motion intent from execution. A pose says where every joint
should end. A motion says which poses to visit, how long each transition takes,
and how long to hold each arrival.

```text
motion/config/poses.yaml
motion/motions/**/*.yaml
            |
            +--> Rust motion library --> oriond --> physical servos
            +--> orion-trajectory -----> Studio and MuJoCo samples
```

## Poses

Each named pose contains exactly Orion's five joints in radians. The values use
the captured physical zero as joint-space zero and must fit inside the accepted
calibration ranges.

## Motions

Absolute keyframes reference named poses. Anchor-relative character clips use
joint offsets and must finish at zero offset. Every keyframe contains:

- `duration`: time spent moving to the pose;
- `arrival`: `through` for continuous motion or `settle` for an intentional stop;
- `hold`: time spent stationary after arrival.

The runtime begins a motion from the latest measured physical position, not
from an assumed previous pose. One continuous piecewise-quintic compiler carries
position, velocity, and acceleration through `through` keyframes, then reaches
zero velocity and acceleration only at `settle` keyframes. Direct interruption
starts from the latest measured position and velocity.

## Physical execution

`oriond --serve` owns the STS3215 connection. Configure and enable it before
sending `--goto` or `--play`. Commands return immediately while movement
continues in the daemon, so scripts must wait for the reported duration or poll
`--status` before issuing another move.

## Simulation execution

MuJoCo asks the Rust `orion-trajectory` binary for the same 50 Hz samples used
by Studio preview and hardware execution. The simulator adds physics, stability,
settling, and torque analysis without owning a second interpolation algorithm.

## Validation

The motion tests check v2 schema, joint order, calibration-derived ranges,
timing, continuous spline dynamics, and report consistency. Rust tests verify
native loading, nonzero velocity at expressive through-keyframes, anchor return,
uniform safe-range scaling, daemon behavior, and hardware/MuJoCo boundaries.
Both suites must pass when a shared pose or motion changes.
