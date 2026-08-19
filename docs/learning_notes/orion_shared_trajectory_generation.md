# Orion Shared Trajectory Generation

This note explains the first Milestone 3 runtime slice: how Orion turns named
keyframes into one smooth desired trajectory that both ROS and native MuJoCo
consume.

## Why This Slice Exists

Milestone 2 shared motion files and target timing, but it did not share the
motion between targets:

```text
ROS joint trajectory controller -> interpolated sparse position points
native MuJoCo player            -> performed its own linear interpolation
```

The two backends agreed on destinations but not on the desired path, velocity,
or acceleration. Position-only ROS waypoints also select linear controller
interpolation, which has discontinuous velocity at waypoint boundaries.

Milestone 3 introduces a new boundary:

```text
motion YAML + named poses
          |
          v
ResolvedTrajectory
    requested destinations and authored timing
          |
          + measured stopped joint state
          + versioned dynamic limits
          |
          v
GeneratedTrajectory
    shared quintic segments and position/velocity/acceleration points
          |
          v
ValidationReport -> ValidatedTrajectory
    complete execution-safety decision
          |
          +-------------------------+
          |                         |
          v                         v
ROS message adapter         native MuJoCo sampler
```

The adapters now translate or sample a desired trajectory. They no longer
choose its interpolation shape.

## Files and Responsibilities

```text
ros2_ws/src/orion_motion/
├── config/motion_limits.yaml
├── orion_motion/
│   ├── trajectory_builder.py
│   ├── trajectory_generator.py
│   ├── trajectory_validator.py
│   ├── ros_state_reader.py
│   ├── ros_motion_player.py
│   └── ros_pose_player.py
└── test/

simulation/mujoco/
└── motion_player.py
```

- `trajectory_builder.py` still resolves semantic pose names and authored
  durations. Its output is a request, not an executable trajectory.
- `trajectory_generator.py` combines that request with a measured start and
  shared limits. It produces smooth, inspectable segments and records analytic
  peak dynamics without making the execution decision.
- `trajectory_validator.py` checks the complete generated path and is the only
  component that can issue a `ValidatedTrajectory` execution capability.
- `ros_state_reader.py` maps `/joint_states` by semantic joint name into
  Orion's canonical order. It requires both position and velocity feedback.
- `ros_motion_player.py` sends generated position, velocity, and acceleration
  boundary points to `joint_trajectory_controller`.
- `simulation/mujoco/motion_player.py` samples the same generated segments at
  the MuJoCo physics timestep.

The generator imports neither ROS nor MuJoCo. This is what makes its desired
motion backend-neutral.

## The Version 2 Limit Contract

`motion_limits.yaml` now distinguishes:

- Mechanical position range from the URDF.
- Operational position range used by the motion system.
- Maximum velocity.
- Maximum acceleration.
- Maximum jerk.
- Maximum cancellation deceleration reserved for the later cancellation slice.
- Maximum absolute velocity accepted as a stopped start.

The current operational positions equal the mechanical ranges because tighter
evidence-based margins do not yet exist. Dynamic values are labelled:

```yaml
applicability: provisional_simulation_only
```

This label matters. The values are conservative development gates for Gazebo
and MuJoCo, not measured STS3215 or physical-lamp safety limits.

The validator ensures that:

- The file uses schema version 2 and explicit derivative units.
- Every canonical joint has the complete limit set.
- Mechanical and operational ranges are finite and ordered.
- Operational positions stay inside mechanical positions.
- Dynamic magnitudes are finite and positive.
- The stopped-start threshold is finite and non-negative.

## Measured Start State

Generation requires complete position and velocity vectors in canonical order:

```text
base_yaw_joint
shoulder_pitch_joint
elbow_pitch_joint
head_roll_joint
head_pitch_joint
```

For live ROS execution, the player subscribes to `/joint_states` and maps
values by name. Array order in the incoming message is irrelevant. The request
is rejected when a required joint or velocity value is missing.

The generator then checks that:

- Every value is finite.
- Every position is in the operational range.
- Every absolute velocity is below the stopped-start threshold.

The generated trajectory begins with an explicit `t = 0` point at the measured
position. Velocity and acceleration are zero because this first implementation
only accepts a stopped start.

Moving-state blending is not approximated. Preemption will first need a
controlled stop, a fresh state reading, and regeneration.

For a dry run there is no simulator feedback, so the user may choose an
explicit named start pose:

```bash
ros2 run orion_motion play_motion look_at_left \
  --dry-run \
  --start-pose attentive
```

That named pose is a preview input only. Normal execution always waits for
measured joint state.

## Quintic Time Scaling

Each transition uses:

```text
u = t / T
s(u) = 10u^3 - 15u^4 + 6u^5
q(t) = q0 + (q1 - q0)s(u)
```

where:

- `q0` is the measured start or preceding keyframe position.
- `q1` is the next keyframe position.
- `T` is the authored transition duration.
- `u` is clamped to the transition interval from zero to one.

The first two derivatives are:

```text
ds/dt  = (30u^2 - 60u^3 + 30u^4) / T
d2s/dt2 = (60u - 180u^2 + 120u^3) / T^2
```

At both ends of a transition:

```text
position     = requested boundary position
velocity     = 0
acceleration = 0
```

That gives position, velocity, and acceleration continuity between transitions
and authored holds. A hold is represented as a constant-position segment with
zero velocity and acceleration.

This profile is jerk-aware, not fully jerk-limited. Jerk is finite and checked,
but it is not continuous into every hold. Physical hardware may later require
an online jerk-limited generator.

## Analytic Dynamic Validation

For this stopped-boundary quintic, the exact peak magnitudes for a joint
displacement `D = |q1 - q0|` are:

```text
peak velocity     = 1.875 D / T
peak acceleration = (10 / sqrt(3)) D / T^2
peak jerk         = 60 D / T^3
```

The generator calculates these values for every joint and transition. The
trajectory validator independently checks them against the configured limits.
A violation reports:

- Destination pose.
- Segment index.
- Joint name.
- Violated limit.
- Calculated peak.
- Allowed value.
- Exact minimum segment duration needed to satisfy that individual limit.

Authored durations are not silently stretched. Timing contributes to
expressive meaning, so a timing change should be a visible motion-authoring
decision.

Validation collects every violation before rejecting the trajectory. See
`orion_trajectory_validation.md` for the report, forbidden-region, and
execution-capability contracts.

## Generated Data

A `GeneratedTrajectory` contains:

- Motion name and description.
- Canonical joint names.
- Boundary points with position, velocity, and acceleration.
- Quintic transition and constant hold segments.
- Per-joint analytic peak dynamics for every transition.
- Total authored duration.

For a one-keyframe motion with a hold, points look like:

```text
t = 0.00   measured start, zero velocity and acceleration
t = 1.50   keyframe arrival, zero velocity and acceleration
t = 2.00   hold end, same state as arrival
```

ROS sends these fields directly in `JointTrajectoryPoint`. Because position,
velocity, and acceleration are all present, the Jazzy trajectory controller
uses quintic spline interpolation.

MuJoCo evaluates the same polynomial at every physics step and sends only the
sampled positions to its position actuators. MuJoCo's measured response can
still differ from the desired path because actuator dynamics and gravity remain
part of the simulation.

## Current Motion-Library Result

Using `attentive` as the stopped start, the provisional limits currently
produce:

| Motion | Result |
|---|---|
| `acknowledge` | Pass |
| `look_at_left` | Pass |
| `look_at_right` | Pass |
| `return_home` | Pass |
| `target_unreachable` | Pass |
| `acknowledge_expressive` | Rejected: aggressive head motion |
| `look_at_left_expressive` | Rejected: aggressive initial turn |
| `target_unreachable_expressive` | Rejected: aggressive reach transition |

This does not mean the expressive concepts are invalid. It means their short
Milestone 2 timings were authored before dynamic validation existed. They need
an explicit retiming pass against the provisional limits.

## Verification

Run the package suite:

```bash
cd /home/mofe/Desktop/dev/orion/ros2_ws/src/orion_motion
source /opt/ros/jazzy/setup.bash
python3 -m pytest -q
```

Run the MuJoCo mapping and shared-trajectory tests:

```bash
cd /home/mofe/Desktop/dev/orion
.venv/bin/python -m unittest -q simulation/mujoco/test_mujoco_backend.py
```

Run a valid headless MuJoCo motion:

```bash
.venv/bin/python simulation/mujoco/motion_player.py \
  look_at_left \
  --start-pose attentive \
  --lead-in 0 \
  --headless
```

The tests cover:

- Versioned limit validation.
- Complete finite measured state.
- Canonical joint-state mapping.
- Stopped-start rejection.
- Exact quintic boundary values.
- Midpoint position, velocity, and acceleration.
- Constant holds.
- Analytic peak dynamics.
- Complete structured dynamic-limit rejection.
- Minimum safe-duration guidance without automatic retiming.
- Continuous forbidden-region intersection.
- Backend rejection of raw, unvalidated trajectories.
- ROS position/velocity/acceleration messages.
- Native MuJoCo consumption of the shared generated trajectory.

## What This Slice Does Not Yet Prove

This slice does not yet implement:

- Result deadlines and settling-based success.
- Action feedback capture.
- Cancellation or preemption.
- Smooth deceleration on cancel.
- Evidence-backed project forbidden regions (the contract and continuous
  checker exist, but the project list is explicitly empty).
- Base-stability checks.
- `mujoco_ros2_control` integration.
- Gazebo-versus-MuJoCo measured trajectory reports.
- Hardware-safe dynamic limits.

Those remain subsequent Milestone 3 slices.
