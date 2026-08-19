# Orion ROS Execution Lifecycle

This note explains Milestone 3 Slice 4: how a validated trajectory becomes a
bounded, observable ROS 2 controller interaction.

## Execution Is Its Own Artifact

The previous client returned a boolean. That lost important distinctions:

```text
goal rejected
path tolerance violated
goal tolerance violated
controller failed
result never arrived
success
```

`execution_types.py` now defines backend-neutral immutable records:

- `JointExecutionState` for one desired, actual, or error state.
- `ExecutionFeedback` for one time-aligned observation.
- `ExecutionStatus` for a specific terminal outcome.
- `ExecutionResult` for the outcome, retained feedback, backend error code,
  and whether cancellation was requested.

These records import neither ROS nor MuJoCo. The ROS adapter converts action
messages into them; the native MuJoCo adapter can later produce the same
evidence directly from physics steps.

## Fresh Measured State

`ros_state_reader.py` records the monotonic receipt time of each newly
delivered `/joint_states` message. The action adapter checks its age after the
action server becomes available and immediately before sending the goal.

This order matters. A sample can be fresh when trajectory generation starts
but become stale during a long server wait. Orion rejects that goal instead of
assuming the generated `t = 0` point still matches the robot.

The thresholds live in the independently versioned `execution_policy.yaml`.
The current maximum age is `0.25 s`. It is a provisional simulation threshold,
not a physical-hardware measurement or joint-dynamics limit.

## Two Tolerance Layers

The same provisional thresholds are applied in two places:

1. Every Orion action goal explicitly carries path, goal-position,
   stopped-velocity, and goal-time tolerances.
2. `orion_controllers.yaml` defines matching controller defaults for other
   callers.

The package test reads both YAML contracts and fails if they diverge.

Current values are:

```text
path position tolerance:  0.20 rad
goal position tolerance:  0.05 rad
stopped velocity:         0.05 rad/s
goal time tolerance:      0.50 s of controller time
```

Non-zero path and goal values make tracking failures enforceable. A non-zero
goal-time tolerance prevents the controller from waiting indefinitely to enter
the goal tolerance.

## Feedback Preservation

The `FollowJointTrajectory` feedback callback supplies:

```text
joint names
desired position / velocity / acceleration
actual position / velocity / acceleration
error position / velocity / acceleration
trajectory-relative time
feedback timestamp
```

The ROS adapter converts every sample into `ExecutionFeedback` and returns the
complete sequence in `ExecutionResult`. It does not replace measured values
with commanded values or reduce the sample to one maximum-error number.

## Finite Result Deadline

Controller trajectory time may be simulation time, while the Python wait uses
wall time. Headless Gazebo on this workstation advances slower than real time,
so equating the two caused a valid two-second simulated trajectory to time out
after 3.5 wall-clock seconds.

The provisional wall deadline is therefore:

```text
authored duration * result_timeout_factor
    + goal_time_tolerance
    + result_timeout_margin
```

With the current factor `5.0`, tolerance `0.5`, and margin `1.0`, a two-second
trajectory has an `11.5 s` wall deadline. This remains finite if simulation
pauses or a result is lost, while allowing slower-than-real-time physics.

If the deadline expires, Orion calls `cancel_goal_async()` and returns
`TIMED_OUT` with `cancel_requested = true`. Slice 4 proves the request is made.
Slice 5 must add bounded deceleration, idempotent cancellation, and stopped
state confirmation before claiming safe cancellation.

## Native MuJoCo Boundary

Native MuJoCo does not have a ROS action server, so it should not imitate ROS
goal handles or controller error codes. It already:

- Requires `ValidatedTrajectory`.
- Samples the same quintic desired path.
- Measures final joint error from MuJoCo state.

Its Slice 6 adapter will add physics-step `ExecutionFeedback`, settling-based
completion, base translation and roll/pitch, contact-loss/tip events, and an
`ExecutionResult` with backend `native_mujoco`. Those records can then be
compared with ROS/Gazebo evidence without pretending the backend mechanisms are
identical.

## Verification

```bash
cd /home/mofe/Desktop/dev/orion/ros2_ws
source /opt/ros/jazzy/setup.bash
colcon test --packages-select orion_motion --event-handlers console_direct+

cd /home/mofe/Desktop/dev/orion
.venv/bin/python -m unittest -q simulation/mujoco/test_mujoco_backend.py
```

For a server-only Gazebo run:

```bash
ros2 launch orion_description gazebo.launch.py \
  gz_args:='-s -r empty.sdf'

ros2 run orion_motion play_motion look_at_left \
  --state-timeout 3 \
  --server-timeout 3
```

The 2026-08-19 live run loaded both controllers, selected spline interpolation,
accepted the action goal, and reported `Goal reached, success!`. Native MuJoCo
regression retained its `0.038004 rad` maximum final error for the same
functional motion.
