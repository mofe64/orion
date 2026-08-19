# Milestone 3 — Motion Quality, Safety, and Simulator Parity

## Document status

- **Status:** Design baseline; implementation not started
- **Created:** 2026-08-19
- **Milestone 2 baseline:** `c4eaca9` (`motion foundation`)
- **Roadmap source:** `docs/Orion Guidebook.md`, section 12

This document turns the guidebook's Milestone 3 direction into an engineering
plan. It records the intended contracts before implementation changes the
motion format, controller configuration, or simulator adapters.

## Objective

Turn Orion's validated keyframe playback into a dependable motion system that:

- Begins from fresh measured joint state without a startup jump.
- Generates smooth, inspectable joint trajectories.
- Rejects trajectories that violate configured position, velocity,
  acceleration, or jerk limits.
- Can be cancelled and preempted predictably.
- Reports desired, executed, and measured motion separately.
- Applies the same motion meaning in Gazebo and MuJoCo.
- Detects known forbidden joint-space regions and simulated tip-risk.

Milestone 3 is complete only when the system demonstrates these properties. A
motion looking smooth in one simulator is not sufficient evidence.

## Milestone boundary

This milestone may change motion generation, validation, execution lifecycle,
controller configuration, simulator adapters, tests, and motion-quality
documentation.

It does not add:

- Task-space inverse kinematics or arbitrary 3D target pointing.
- Perception, cameras, voice, or an LLM.
- Physical-servo control.
- Collision-free path planning for arbitrary environments.
- A general expression optimiser.
- A claim that simulator-derived limits are safe for physical hardware.

Task-space control remains Milestone 4. Physical limits must later be measured
on the LeLamp-compatible prototype before they are used as hardware safety
limits.

## Verified starting point

Milestone 2 currently provides:

- Complete named poses and pose-referenced keyframe motions.
- Mechanical position-limit and schema validation.
- A backend-neutral resolved-keyframe representation.
- A ROS 2 `FollowJointTrajectory` action client for Gazebo.
- A native MuJoCo stepping adapter using the same motion files.
- Functional and expressive A/B motion pairs.
- 43 passing `orion_motion` tests and one passing MuJoCo mapping test.

The baseline also exposes the work Milestone 3 must address:

1. The ROS message contains position-only waypoints. The ROS 2 joint trajectory
   controller therefore uses position-continuous linear interpolation, whose
   velocity changes discontinuously at waypoints.
2. The native MuJoCo player performs its own linear interpolation from a
   measured start for every segment. ROS and MuJoCo therefore share keyframes,
   but do not yet share one generated trajectory.
3. The ROS client waits for a result without an execution deadline, feedback
   callback, cancellation API, or explicit preemption policy.
4. Durations are accepted without proving they satisfy dynamic limits.
5. Completion is not independently checked against measured position,
   velocity, and a settling interval. In the baseline headless MuJoCo
   acknowledgement run, playback time elapsed with a maximum final joint error
   of approximately `0.0366 rad`.
6. The package has mechanical joint limits but no separate operational dynamic
   limits, forbidden-region rules, or base-stability criteria.
7. `mujoco_ros2_control` is not installed in the current development
   environment, so shared ROS controller execution in MuJoCo requires an
   explicit dependency and integration step.

## Concepts to learn

### Keyframes are not trajectories

A keyframe states a desired joint pose and arrival time. It does not define all
intermediate positions or prove that the transition is dynamically safe.

Milestone 3 must preserve five different artifacts:

```text
RequestedMotion
    semantic pose names, durations, and holds
        |
        v
GeneratedTrajectory
    measured start plus continuous position/velocity/acceleration segments
        |
        v
ValidationReport + ValidatedTrajectory
    limit, timing, forbidden-region, and stability evidence
        |
        v
ExecutionRecord
    accepted goal, backend, lifecycle transitions, feedback, and result
        |
        v
MeasuredTrajectory
    timestamped joint and base-state observations
```

The types should not be aliases for the same object. In particular, only a
successful validation operation may produce a `ValidatedTrajectory` for normal
execution.

### Continuity has levels

- Position continuity prevents an instantaneous joint-position jump.
- Velocity continuity prevents an instantaneous speed change.
- Acceleration continuity prevents an instantaneous acceleration change.
- Jerk is the rate of acceleration change.

The ROS 2 joint trajectory controller selects linear, cubic, or quintic spline
interpolation based on whether waypoints contain position only,
position-and-velocity, or position-velocity-and-acceleration respectively.
Milestone 3 will supply all three fields and use quintic segments.

This choice makes Orion acceleration-continuous within and across correctly
matched segments. It is **jerk-aware**, because peak jerk is calculated and
limited, but it is not a claim of fully jerk-limited motion. A later physical
hardware milestone may require an online jerk-limited generator.

### Commanded state is not measured state

A simulator or actuator can lag, overshoot, saturate, or fail. Therefore action
acceptance and elapsed trajectory time are not enough to prove success. Orion
must preserve desired-versus-actual feedback and apply explicit goal and
stopped-velocity tolerances.

### Cancellation is not emergency stopping

- **Cancellation** requests a controlled deceleration and stopped hold.
- **Preemption** cancels the active motion, reaches a stopped state, then
  regenerates the replacement from newly measured state.
- **Safe return-to-rest** is a new validated motion after stopping, and only
  runs when the fault policy allows movement.
- **Emergency stop** belongs to a higher-priority safety path. It must never
  trigger an automatic return motion.

## Architecture decisions

### 1. Orion owns the desired motion shape

The backend-neutral `orion_motion` library will generate quintic segments. A
segment is defined by start and end time and by position, velocity, and
acceleration at both boundaries.

For the first implementation:

- Every existing keyframe has a non-zero hold.
- Held keyframes use zero velocity and zero acceleration.
- A normal motion begins only when measured joint velocity is below the
  configured stopped threshold.
- The generated trajectory contains an explicit measured start point at
  `t = 0` with zero velocity and acceleration.
- Each following pose arrival and hold endpoint contains position, zero
  velocity, and zero acceleration.

Those constraints produce one unambiguous quintic polynomial per transition
and constant-position hold segments. The ROS adapter sends the boundary states
to `joint_trajectory_controller`; the native MuJoCo adapter evaluates the same
segment definition at its physics timestep.

Starting from a significantly moving state is rejected in the first slice. A
preemption must first perform a controlled stop. Supporting non-zero measured
start velocity can be added only with tests proving continuity and safe limits.

### 2. Durations are validated, not silently changed

For zero velocity and acceleration at both ends, each joint follows the
standard quintic time-scaling function:

```text
u = t / T
s(u) = 10u^3 - 15u^4 + 6u^5
q(t) = q0 + (q1 - q0)s(u)
```

The generator will calculate analytic peak velocity, acceleration, and jerk
for every joint and segment. If a requested duration violates a configured
limit, generation returns a structured rejection explaining the joint,
segment, measured peak, and allowed limit.

Milestone 3 will not silently stretch authored timing at first. Timing is part
of expressive intent, so an automatic change could alter a behaviour's
meaning. A future opt-in retiming mode may report the required duration and let
the caller choose whether to accept it.

### 3. Mechanical and operational limits remain distinct

`config/motion_limits.yaml` will remain the authoritative joint contract, but
its schema will be deliberately migrated from format version 1. For each joint
it must distinguish:

- Mechanical lower and upper position limits.
- Tighter operational lower and upper position limits.
- Maximum operational velocity.
- Maximum operational acceleration.
- Maximum operational jerk.
- Maximum cancellation deceleration.
- Provenance and applicability, such as `provisional_simulation`.

The current URDF velocity value of `10 rad/s` must not be treated as an
evidence-based safe operating limit. Initial dynamic limits will be explicitly
labelled provisional and selected through simulator measurements. Physical
hardware limits require later bench evidence.

### 4. Measured state is mandatory and must be fresh

Before generation, the executor must obtain all five canonical joints from one
state snapshot and validate:

- Every required joint is present exactly once.
- Values are finite.
- Position is within mechanical and operational limits.
- The message timestamp is not older than the configured state timeout.
- Velocity is present and below the normal-start threshold.

Missing, stale, partial, or moving state rejects normal motion. The executor
must not substitute a named pose or the last commanded position.

### 5. Preemption uses stop-and-regenerate

Only one motion may own the controller at a time. If a replacement request
arrives:

```text
active motion
    -> request cancellation
    -> controlled deceleration
    -> confirm stopped state
    -> read a fresh complete state
    -> generate and validate replacement
    -> execute replacement
```

One pending replacement is retained using a latest-request-wins policy. This
is intentionally less fluid than blending trajectories, but it is predictable
and preserves measured-start continuity. Seamless moving-state blending is
deferred until the stopped-state path is proven.

Directly sending a second action goal is not Orion's preemption policy: the ROS
controller replaces the old trajectory, which can discard assumptions used by
Orion's validation.

### 6. Completion requires settling evidence

A motion succeeds only when:

- The controller or backend accepted the trajectory.
- No path tolerance, safety rule, or timeout failed.
- Every joint is within its configured goal-position tolerance.
- Every joint velocity is below its stopped-velocity tolerance.
- Both conditions remain true for the configured settle duration.
- Completion occurs before the execution deadline.

The client deadline is separate from the controller's `cmd_timeout`. It should
be computed from the generated duration, configured goal-time tolerance,
settle duration, and a bounded communication margin.

### 7. Cancellation decelerates before holding

The installed Jazzy joint trajectory controller supports
`constraints.decelerate_on_cancel` and per-joint
`max_deceleration_on_cancel`, provided velocity state is available. Orion will
configure and test this behaviour in Gazebo rather than relying on its default
immediate hold.

The native MuJoCo adapter must implement and test the same high-level outcome:
bounded deceleration to zero velocity followed by a hold. Later in Milestone 3,
MuJoCo should be evaluated through `mujoco_ros2_control` so both simulators can
use the same ROS controller stack. The native runner remains useful as an
independent model and regression tool.

### 8. Stability is measured in a free-standing simulation

Known unsafe joint-space regions and dynamic base stability are different
checks:

- `config/forbidden_regions.yaml` will describe disallowed combinations of
  joint ranges. Rules apply to positions, not pose names, so renaming or
  reaching the same posture through another motion cannot bypass them.
- MuJoCo playback will record base translation, base roll/pitch, height,
  contact state, and peak joint dynamics for every generated trajectory.
- Provisional thresholds will be stored in configuration with their rationale.
- A trajectory that crosses a forbidden region is rejected before execution.
- A trajectory that exceeds simulated stability thresholds is classified
  unsafe and included in the validation report.

Simulator stability evidence is a development gate, not certification of
physical safety.

## Proposed package changes

No new ROS package is needed yet. The boundary belongs inside
`ros2_ws/src/orion_motion` until a second package has a distinct responsibility.

```text
orion_motion/
├── config/
│   ├── motion_limits.yaml          # migrated joint and dynamic-limit contract
│   ├── forbidden_regions.yaml      # joint-space exclusion rules
│   └── stability_limits.yaml       # provisional simulator thresholds
├── orion_motion/
│   ├── trajectory_builder.py       # requested keyframes only
│   ├── trajectory_generator.py     # measured-start quintic generation
│   ├── trajectory_validator.py     # dynamic and forbidden-region checks
│   ├── execution_types.py          # feedback, result, and lifecycle records
│   ├── ros_state_reader.py         # complete fresh joint-state snapshot
│   └── ros_motion_player.py        # action lifecycle adapter
└── test/
    ├── test_trajectory_generator.py
    ├── test_trajectory_validator.py
    └── test_motion_lifecycle.py

simulation/mujoco/
├── motion_player.py                # consume GeneratedTrajectory
├── stability_monitor.py            # base and contact metrics
└── parity_runner.py                 # repeatable playback and report data
```

Names are proposed boundaries, not permission to create every file at once.
Each implementation slice should introduce only the files it needs.

## Execution lifecycle

The executor uses explicit states:

```text
IDLE
  -> ACQUIRING_STATE
  -> GENERATING
  -> VALIDATING
  -> EXECUTING
  -> SETTLING
  -> SUCCEEDED

EXECUTING
  -> CANCELLING
  -> STOPPING
  -> CANCELLED

EXECUTING + replacement request
  -> CANCELLING
  -> STOPPING
  -> ACQUIRING_STATE
  -> GENERATING replacement

Any pre-execution failure
  -> REJECTED

Any bounded wait that expires
  -> TIMED_OUT

Any controller/backend failure
  -> FAILED
```

Terminal results must distinguish at least:

```text
SUCCEEDED
REJECTED_MALFORMED
REJECTED_STALE_STATE
REJECTED_MOVING_START
REJECTED_DYNAMIC_LIMIT
REJECTED_FORBIDDEN_REGION
CANCELLED
PREEMPTED
TIMED_OUT
CONTROLLER_REJECTED
PATH_TOLERANCE_VIOLATED
GOAL_TOLERANCE_VIOLATED
BACKEND_FAILED
UNSAFE_STABILITY_RESULT
```

Each result includes a human-readable explanation and structured fields such
as the affected joint, limit, measured value, motion name, and backend.

## Implementation sequence

### Slice 0 — Preserve and document the baseline

1. Keep `c4eaca9` as the Milestone 2 checkpoint.
2. Correct the guidebook's stale current-position section.
3. Add this design document.
4. Do not change runtime semantics in the documentation commit.

### Slice 1 — Dynamic-limit contract

Files involved:

- `config/motion_limits.yaml`
- `motion_validator.py`
- Existing validator tests

Work:

1. Design and test the versioned schema migration.
2. Add distinct operational position, velocity, acceleration, jerk, and cancel
   deceleration limits with provenance.
3. Reject missing, non-finite, non-positive, or internally inconsistent limits.
4. Keep canonical joint names and order unchanged.

Verification:

- Old schema rejection explains the required migration.
- Every joint has all required limit fields.
- Operational positions are contained by mechanical positions.
- No runtime or simulator code is changed in this slice.

### Slice 2 — Pure quintic trajectory generator

Files involved:

- New `trajectory_generator.py`
- `trajectory_builder.py`
- New unit tests

Work:

1. Add measured-start and generated-segment data types.
2. Generate explicit position, velocity, and acceleration boundary states.
3. Evaluate segment position, velocity, acceleration, and jerk at arbitrary
   elapsed time.
4. Add analytic peak-dynamic calculations.
5. Preserve authored arrival and hold times.

Verification:

- Exact boundary conditions at segment start and end.
- Position, velocity, and acceleration continuity.
- Constant positions and zero derivatives during holds.
- Canonical joint ordering.
- Deterministic output independent of ROS and simulators.

### Slice 3 — Automated trajectory validation

Files involved:

- New `trajectory_validator.py`
- New `config/forbidden_regions.yaml`
- New validation tests

Work:

1. Validate monotonic timing and complete finite state vectors.
2. Check mechanical and operational position limits.
3. Check analytic peak velocity, acceleration, and jerk.
4. Check all segment samples and extrema against forbidden regions.
5. Produce a structured `ValidationReport`.

Verification:

- A known-valid slow motion passes.
- Deliberately short durations fail with joint-specific diagnostics.
- A trajectory crossing a forbidden region fails even when both endpoints are
  outside it.
- Only a passing report can construct an executable validated trajectory.

### Slice 4 — Measured-state ROS execution

Files involved:

- New `ros_state_reader.py`
- `ros_motion_player.py`
- `ros_pose_player.py`
- ROS conversion and lifecycle tests

Work:

1. Read a complete, fresh five-joint snapshot.
2. Reject moving starts above the stopped threshold.
3. Generate and validate after state acquisition.
4. Send position, velocity, and acceleration at every trajectory point.
5. Add goal tolerances, feedback capture, and a finite result deadline.
6. Configure non-zero controller goal-time and path/goal tolerances.

Verification:

- First command point exactly matches measured state.
- No goal is sent for stale, partial, moving, or unsafe input.
- Feedback preserves desired, actual, and error fields.
- A goal that never returns times out and requests cancellation.
- Goal and path tolerance failures remain distinguishable.

### Slice 5 — Cancellation and preemption

Files involved:

- ROS executor/lifecycle code
- `orion_controllers.yaml`
- Lifecycle tests and Gazebo integration tests

Work:

1. Configure bounded deceleration on cancel for every joint.
2. Implement idempotent cancellation.
3. Confirm stopped state before returning a cancelled result.
4. Implement one-slot latest-wins preemption.
5. Regenerate the replacement from a fresh stopped state.
6. Keep return-to-rest as a separate policy-controlled motion.

Verification:

- Cancel during every segment and hold.
- Repeated cancel requests do not create multiple stop operations.
- Replacement never reuses the old motion's expected state.
- Emergency-stop simulation does not initiate return-to-rest.

### Slice 6 — MuJoCo stability and parity instrumentation

Files involved:

- Native MuJoCo player
- New stability monitor and parity runner
- Stability configuration and tests

Work:

1. Execute the shared generated segments in native MuJoCo.
2. Capture timestamped desired and measured joint state.
3. Capture free-base pose and contact metrics.
4. Add completion tolerances and settle duration.
5. Generate machine-readable run data and a human-readable summary.

Verification:

- The current time-only completion defect becomes a failed settling result when
  final error or velocity exceeds tolerance.
- A stable slow trajectory passes.
- An intentionally aggressive test trajectory is detected or explicitly
  classified unsafe.
- Closing the viewer produces cancellation, not success.

### Slice 7 — Shared ROS controller stack in MuJoCo

Work:

1. Install and validate the Jazzy `mujoco_ros2_control` demo separately.
2. Connect Orion's existing native MJCF scene to
   `mujoco_ros2_control/MujocoSystemInterface`.
3. Expose the five semantic position command interfaces and position/velocity
   state interfaces.
4. Keep the free joint unactuated.
5. Run the same `FollowJointTrajectory` client and controller configuration
   used by Gazebo.
6. Retain the independent native runner for model and stability regression.

This slice requires a new system dependency and should not be mixed into the
pure trajectory-generation changes.

### Slice 8 — Cross-simulator report and closeout

For every named functional and expressive motion:

1. Start both simulators from the same canonical pose and stopped state.
2. Use the same motion file, generated trajectory, limits, and tolerances.
3. Record desired and measured positions and velocities.
4. Compare semantic direction, timing, peak dynamics, final error, settling,
   cancellation, and stability.
5. Publish `docs/validation/milestone_3_motion_validation.md` with the results,
   environment versions, limitations, and failures.

## Validation matrix

| Layer | Evidence | Primary checks |
|---|---|---|
| Loader/schema | Unit tests | Safe YAML, version, required fields |
| Requested motion | Unit tests | Pose names, durations, holds, joint completeness |
| Generator | Unit and property-style tests | Boundary values, derivatives, continuity, timing |
| Validator | Unit tests | Dynamic peaks, forbidden regions, structured failures |
| ROS adapter | Unit/fake-action tests | State freshness, messages, feedback, timeout, cancellation |
| Gazebo | Integration runs | Startup continuity, tracking, cancel/preempt, settling |
| Native MuJoCo | Integration runs | Shared desired path, tracking, base stability, settling |
| MuJoCo ROS control | Integration runs | Same action/controller contract as Gazebo |
| Parity | Reported comparison | Names, signs, timing, target, limits, outcome semantics |

## Required validation report fields

Each simulator run should record:

```text
motion name and source hash
backend and software versions
joint contract and limit-config hash
requested keyframes
measured start state and age
generated duration and segment count
per-joint peak desired velocity, acceleration, and jerk
per-joint maximum tracking error
per-joint final position and velocity error
settling time
action/backend result
cancellation stopping time and distance, when applicable
maximum base translation and roll/pitch
contact-loss or tip event
validation warnings and failures
```

Pass thresholds must come from versioned configuration, not from assertions
scattered through simulator scripts.

## Failure modes and required responses

| Failure | Required response |
|---|---|
| Motion or limits YAML malformed | Reject before state acquisition; identify file and field |
| Unknown or partial joint set | Reject; never infer array positions |
| State missing or stale | Reject with state-age diagnostic |
| Robot already moving | Reject normal start or complete controlled stop first |
| Requested duration exceeds a dynamic limit | Reject with joint, segment, peak, and limit |
| Trajectory enters a forbidden region | Reject before execution |
| Action server unavailable | Time out without sending movement |
| Controller rejects goal | Report controller code and text; remain stopped |
| Path tolerance violated | Cancel/decelerate, hold, and report affected joint |
| Goal tolerance not reached | Time out, cancel/decelerate, hold, and report final error |
| Feedback stops arriving | Treat measured state as stale; cancel and report failure |
| User cancels | Decelerate, confirm stopped, return `CANCELLED` |
| New motion preempts active motion | Stop, reacquire state, regenerate, execute newest request |
| Stability threshold exceeded in simulation | Mark unsafe; do not include in normal motion library |
| Emergency stop | Stop through the safety path; never return to rest automatically |

## Open decisions requiring evidence

The design deliberately does not invent these values:

- Operational position margins inside the mechanical limits.
- Per-joint velocity, acceleration, jerk, and cancel-deceleration limits.
- State freshness timeout.
- Normal-start stopped-velocity threshold.
- Path and goal position tolerances.
- Goal velocity tolerance and settle duration.
- Execution communication margin.
- Base translation, roll/pitch, height, and contact-loss thresholds.
- Exact forbidden joint-space regions.

They should be selected through slow simulator sweeps, documented rationale,
and later revalidated on physical hardware. Until then they must be labelled
`provisional_simulation`, not `safe_hardware`.

One product-level question also remains: after an ordinary cancelled behaviour,
should Orion hold its stopped position or return to `home` after a delay? The
motion layer can support either, but the default policy belongs to behaviour
and safety design. Milestone 3 should default to holding and reporting rather
than initiating unrequested motion.

## Exit-criteria traceability

| Guidebook exit criterion | Planned evidence |
|---|---|
| Motions begin from measured current position | Fresh-state test and first-point equality in both simulators |
| No startup position discontinuity | Desired/measured trace at `t = 0` and continuity tests |
| Motions can be stopped safely | Cancel-deceleration and stopped-state integration tests |
| Unsafe or malformed motions are rejected | Schema, dynamic-limit, forbidden-region, and stability tests |
| Named motions remain portable | Same-source parity matrix for Gazebo and MuJoCo |
| Tip-risk motion is detected or marked unsafe | Free-base stability metrics and aggressive negative test |

## Lessons-learned section for closeout

At Milestone 3 completion, append evidence-based notes covering:

- Which interpolation assumptions matched both simulators.
- Which dynamic limits required adjustment and why.
- Where desired and measured trajectories differed most.
- Whether controlled cancellation was sufficiently smooth.
- Which base-stability indicators predicted tipping reliably.
- Which decisions must be revisited before physical hardware.

## References

- [Orion Guidebook](../Orion%20Guidebook.md)
- [Orion motion-foundation learning note](../learning_notes/orion_motion_foundation.md)
- [Orion future control architecture](../orion_control_architecture.md)
- [ROS 2 Jazzy joint trajectory controller](https://control.ros.org/jazzy/doc/ros2_controllers/joint_trajectory_controller/doc/userdoc.html)
- [ROS 2 Jazzy trajectory representation and interpolation](https://control.ros.org/jazzy/doc/ros2_controllers/joint_trajectory_controller/doc/trajectory.html)
- [ROS 2 Jazzy joint trajectory controller parameters](https://control.ros.org/jazzy/doc/ros2_controllers/joint_trajectory_controller/doc/parameters.html)
- [ROS 2 Jazzy MuJoCo control integration](https://control.ros.org/jazzy/doc/mujoco_ros2_control/doc/index.html)
