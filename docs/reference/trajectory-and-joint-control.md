# Trajectory and joint-control reference

Orion's Rust runtime converts each accepted semantic motion into five
synchronized STS3215 goal positions. The trajectory compiler constructs the
continuous joint paths, `RuntimeCore` samples them at 50 Hz, and the hardware
driver converts radians to calibrated servo values.

## Constants and authorities

| Concern | Authority |
| --- | --- |
| Command rate | `50 Hz` (`20 ms` period) in `RuntimeCore` and the daemon loop |
| Position limits | Active calibration loaded by the hardware driver |
| Offline position limits | `simulation/mujoco/config/servo_calibration.json` |
| Encoder resolution | `4096` counts/revolution |
| Motor speed ceiling | `5.445427266... rad/s`, the 7.4 V STS3215 52 RPM no-load specification |
| Completion position tolerance | `0.05 rad` maximum joint error |
| Completion velocity tolerance | `0.05 rad/s` maximum measured joint speed |
| Required settled duration | `0.25 s` continuously within both tolerances |
| Settling timeout | `2.0 s` after authored trajectory completion |
| Simulation reporting policy | `motion/config/stability_limits.yaml`; diagnostic, not a runtime gate |

The driver reports voltage, current, temperature, and status telemetry.
Position calibration and the motor profile remain the command authorities.

## Compilation entry points

### One-target `goto`

`JointTrajectory::with_start_velocity_calibrated` wraps one target as a single
`settle` waypoint. It uses the same quintic compiler, speed retiming, measured
start velocity, and calibration-safe interruption path as a multi-keyframe
motion.

### Authored motion

`MotionSequence::compile_scaled_calibrated`:

1. resolves every keyframe target from its absolute pose or relative anchor;
2. applies the one uniform relative amplitude scale;
3. converts keyframe arrival and marker data to `TrajectoryWaypoint` values;
4. invokes `CompiledTrajectory::compile_calibrated`; and
5. retains keyframe and marker queries for runtime and scene status.

### Character-owned generated motion

`RuntimeCore::play_generated_anchored_relative` accepts an in-memory
`MotionDefinition`, but applies the same contract as a loaded relative clip:

- motion space must be `anchor_relative`;
- `return_to_anchor` must be true;
- the final keyframe must be one zero-offset `settle`;
- every resolved target must pass driver validation; and
- compilation uses measured start state, immutable anchor, uniform scale,
  calibration, and the STS3215 speed ceiling.

Speech uses this entry point; it does not bypass the normal compiler.

## Input normalization

The compiler requires:

- a non-empty name;
- one complete finite start position and start velocity for the same joints;
- at least one complete finite waypoint;
- positive finite movement durations;
- finite non-negative holds attached only to `settle` arrivals; and
- a positive finite speed ceiling.

Calibrated compilation additionally requires one unique, finite, ordered
`lower_rad < upper_rad` range for every joint and a start position inside each
range.

## Quintic segment model

For each joint and segment, the compiler constructs:

```text
p(t) = c0 + c1 t + c2 t² + c3 t³ + c4 t⁴ + c5 t⁵
```

The six coefficients satisfy six endpoint constraints:

```text
p(0) = p0       p(T) = p1
v(0) = v0       v(T) = v1
a(0) = a0       a(T) = a1
```

Because adjacent segments use the same waypoint position, velocity, and
acceleration, `through` boundaries are C2-continuous: position, first
derivative, and second derivative match at the join.

The compiler keeps exact target positions. It changes derivative values and
segment durations, not authored keyframe positions.

## Duration policy

The initial compiled duration for each authored segment is:

```text
through: authored_duration / style.tempo

settle:  authored_duration / style.tempo
         × (0.85 + 0.30 × style.settle_character)
```

A hold is appended after arrival and does not participate in polynomial
motion. Sampling during a hold returns the exact target with zero velocity and
acceleration.

Speed retiming may lengthen individual movement segments after this style
transformation.

## Internal derivatives

Let `before` and `after` be the average slopes of the adjacent authored
segments for one joint.

### Start point

- position is the latest measured, calibration-clamped position;
- velocity is the latest measured velocity, subject to calibrated interruption
  protection; and
- acceleration starts at zero.

### Final point and settle arrivals

Velocity and acceleration are zero at the final waypoint and after every
`settle` arrival.

### Through arrivals

If adjacent slopes change sign or either is zero, the waypoint velocity is
zero. This represents an instantaneous direction reversal, not a hold.

Otherwise the compiler calculates a duration-weighted slope, bounds its
magnitude to three times the smaller adjacent slope, and multiplies it by
style tangent tension and joint-lag character.

The compiler bases through acceleration on the change between adjacent slopes
over their combined duration, then scales it by tangent tension, joint-lag
character, and overshoot character.

Joint-lag character uses the ordered chain:

```text
base yaw → shoulder pitch → elbow pitch → head roll → head pitch
```

The configured lag reduces derivative magnitude progressively across that
order. It changes how joints travel through shared drawings; it does not delay
servo packets or change the 50 Hz clock. Explicit head-first timing, such as
speech body follow-through, is represented with additional semantic drawings.

## Unrequested overshoot control

For each segment and joint, the compiler samples the quintic at 80 subdivisions
and compares its position with the closed interval between the segment's two
authored endpoints.

If a polynomial exits that interval, the compiler halves only the velocity and
acceleration values bordering that segment and joint. It repeats this local
process up to eight passes.

Local attenuation matters. Flattening one joint's derivatives globally would
allow a difficult turn in one place to create stopped-looking movement in an
unrelated part of a long performance.

An authored overshoot pose remains an endpoint, and the trajectory reaches it
exactly. The overshoot guard prevents additional polynomial overshoot between
authored endpoints.

## Motor-speed retiming

After compiling a candidate, the runtime samples every segment's joint
velocity at 81 points. If a segment exceeds the STS3215 ceiling, only that
segment's duration is multiplied by:

```text
(measured_peak / maximum_velocity) × 1.015
```

The compiler rebuilds derivatives and polynomials and repeats for up to 12
iterations. It rejects the trajectory if the final peak remains more than
0.1% above the ceiling.

Retiming changes marker arrival times because markers belong to compiled
keyframes. Scenes ask the active motion whether a marker has been reached,
which keeps light and audio synchronized with the stretched motion.

## Calibration-safe interruption

Starting from measured velocity preserves physical continuity, but noisy or
high telemetry near a joint boundary can make a polynomial leave calibration.
`compile_calibrated` handles that case without discarding all velocity:

1. Validate that the measured start is inside every range.
2. Bound any measured speed above the motor ceiling to 95% of that ceiling,
   preserving direction.
3. Compile the full candidate.
4. Sample it at the actual 50 Hz command rate.
5. Identify only joints whose samples leave calibration.
6. Halve only those joints' start velocities.
7. Recompile, for at most 18 iterations.

Safe joint velocities that do not cause a boundary violation remain intact.
Targets, styles, and other joints do not change. Failure to find a safe blend
is a rejected motion, never a clipped command.

## Calibration and radians conversion

Each joint calibration contains:

- semantic joint name;
- unique servo ID;
- raw encoder value corresponding to joint-space zero;
- encoder direction, `+1` or `-1`; and
- safe minimum and maximum raw deltas around neutral.

The software converts safe raw deltas to an ordered radian range using the
4096-count encoder. The driver converts a commanded radian value as follows:

```text
steps_per_radian = 4096 / (2π)
delta = round(radians × steps_per_radian) × encoder_direction
raw_goal = (neutral_raw + delta) modulo 4096
```

The driver rejects a delta outside its calibrated range before producing a raw
goal. It also requires a command for exactly all five joints.

Feedback conversion unwraps the raw value to the nearest signed half-turn from
neutral and divides by encoder direction and `steps_per_radian`. Velocity uses
the STS3215 present-speed conversion and the same encoder direction.

## Hardware preparation and torque activation

The STS3215 driver has three distinct stages.

### Connect and validate

With torque off, it opens the serial port and verifies for every configured
servo:

- unique ID and joint name;
- STS3215 model number;
- torque disabled;
- zero fault status; and
- the same firmware version across all five devices.

### Apply the Orion profile

The driver applies and reads back return delay, operating mode, direction,
PID coefficients, maximum acceleration, and runtime acceleration. Persistent
register writes unlock and relock EEPROM. Elbow pitch and head pitch use their
commissioned gravity-load proportional gains.

The servo acceleration registers shape the actuator's local response. They do
not replace the host trajectory or define a second motion plan.

### Activate

Immediately before torque-on, the driver synchronously reads present state,
writes each present encoder position into its goal register, verifies those
goals, and only then enables torque. This ordering prevents torque activation
from snapping toward a stale target.

Deactivation disables torque on every configured ID. Dropping the hardware
driver also attempts torque-off before closing the serial port.

## Synchronized servo I/O

The physical transport uses the STS3215 serial protocol through `rustypot`.
One synchronized feedback read requests the 15-byte state block from all five
IDs. It decodes position, sign-magnitude velocity and current, voltage,
temperature, and status.

One synchronized goal write sends the five two-byte positions to goal register
address 42. The trajectory loop does not issue five independently timed
position writes.

Register-level operations remain private to `Sts3215Driver` and
`Sts3215Transport`. Network and character layers cannot address registers.

## The 50 Hz daemon cycle

The outer service loop advances a monotonic deadline by 20 ms each iteration.
Its order is:

1. `RuntimeCore::tick`
2. scene coordinator tick
3. speech coordinator tick
4. speaking-energy light update
5. character coordinator tick
6. background character light update when no foreground owner exists
7. pending Unix command handling
8. sleep until the next deadline

Inside `RuntimeCore::tick`:

```text
read synchronized feedback
        │
        ├─ active MotionSequence? ─ sample positions + markers
        │
        └─ active JointTrajectory? ─ sample positions
        │
write synchronized goals
update executing progress
        │
authored duration complete?
        ├─ no  → publish snapshot
        └─ yes → enter measured settling → publish snapshot
```

The clock is supplied to `RuntimeCore`, which makes lifecycle and trajectory
tests deterministic. Hardware uses `Instant`; tests can advance time directly.

## Movement lifecycle

```text
executing ── authored samples exhausted ──▶ settling
    │                                         │
    │ stop                                    ├─ stable within tolerance ─▶ completed
    ▼                                         └─ timeout ─────────────────▶ timed_out
cancelled
```

`executing` tracks the compiled trajectory. `settling` tracks physical
feedback against the final target. Completion requires the maximum absolute
joint error and maximum absolute measured velocity to remain within tolerance
for the full settle window.

The daemon returns to `holding` for every terminal movement phase. Disabling
torque is a separate command and cancels any active movement first.

## Marker and status behavior

`MotionSequence` exposes:

- active keyframe label and index;
- total keyframe count;
- progress through total compiled duration;
- compiled marker arrival times; and
- all markers reached at the sampled elapsed time.

Movement run IDs are daemon-local and reset at restart. Status retains only
the active run and the most recent terminal run. Clients must retain the
returned run ID and follow that specific ID; status is not an event database.

## Runtime validation and failure modes

| Boundary | Rejection or terminal behavior |
| --- | --- |
| Asset load | Malformed schema, unknown fields, invalid references, or duplicate names reject the catalog |
| Runtime startup/reload | Absolute targets outside active calibration reject the catalog transaction |
| Relative instantiation | Missing anchor joints, missing limits, or invalid final zero settle reject the clip |
| Trajectory compile | Invalid maps, durations, holds, limits, speed, overshoot, or safe interruption reject movement before start |
| Driver encode | Missing/non-finite/out-of-range commands reject before serial write |
| Movement execution | Driver read/write errors propagate as runtime errors; no completion is claimed |
| Settling | Failure to remain within measured tolerance becomes `timed_out` |
| Cancellation | Compiled trajectory is dropped and the run becomes `cancelled`; holding torque remains on |

## Shared hardware and simulation path

`RuntimeDriver` is expressed in calibrated joint radians:

```text
apply_servo_profile
activate / deactivate
read
write
joint_limits
validate_positions
clamp_positions_to_safe_range
```

The physical driver converts radians to STS3215 packets. `MujocoDriver`
exchanges radians with the simulator. Everything above that boundary—asset
loading, trajectory compilation, timing, markers, run IDs, cancellation, and
settling—uses the same Rust code.

The `orion-trajectory` binary also calls the same compiler to emit portable
50 Hz preview samples. Studio and offline tools consume those samples rather
than reimplementing interpolation.

## Verification map

| Property | Primary automated evidence |
| --- | --- |
| Complete schema and catalog | `pose`, `motion`, and `scene` loader tests |
| C2 continuity at `through` | `trajectory::keeps_position_velocity_and_acceleration_continuous_through_keyframes` |
| Exact settle derivatives | trajectory and motion-sequence sampling tests |
| Segment speed ceiling | `trajectory::retimes_fast_segments_to_the_sts3215_ceiling_without_extra_overshoot` |
| Measured interruption continuity | interruption trajectory tests |
| Calibration-safe interruption | calibrated interruption tests |
| Relative scaling and anchor return | built-in relative clip tests |
| 50 Hz movement lifecycle | daemon state-machine tests |
| Shared MuJoCo execution | native MuJoCo runtime test |
| Radians/raw and synchronized transport | driver and transport tests |

Automated evidence is necessary but not sufficient for animation quality. Use
the physical acceptance guide to evaluate silhouette, perceived timing,
mechanical sound, cable behavior, and appeal.
