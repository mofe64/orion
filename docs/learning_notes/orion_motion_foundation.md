# Orion Motion Foundation

> **Milestone 3 update:** Orion now generates a shared measured-start quintic
> trajectory before ROS or MuJoCo execution. The Milestone 2 representation
> described below remains the requested-keyframe layer. See
> [Shared Trajectory Generation](orion_shared_trajectory_generation.md) for the
> current generation and dynamic-validation path.

This note explains the Milestone 2 motion system from its data files to its
simulator adapters. The goal is to make the subsystem reproducible, not merely
to record commands that happen to work.

## What Milestone 2 Builds

Milestone 2 answers one question:

> How can Orion describe, validate, and execute poses and timed animations
> without making those definitions depend on Gazebo or MuJoCo?

The answer is a layered pipeline:

```text
poses.yaml + motion YAML + motion_limits.yaml
                    |
                    v
             safe YAML loader
                    |
                    v
                validators
                    |
                    v
        backend-neutral trajectory builder
                    |
          +---------+---------+
          |                   |
          v                   v
   ROS action adapter     MuJoCo adapter
          |                   |
          v                   v
      Gazebo now         native MuJoCo now
      hardware later     ROS adapter later
```

The files above the split do not import ROS, Gazebo, or MuJoCo. This is what
"simulator-independent" means in Orion: the meaning of a pose or animation is
shared, while each backend owns only the mechanics of executing it.

## Package Map

```text
ros2_ws/src/orion_motion/
├── config/
│   ├── motion_limits.yaml
│   └── poses.yaml
├── motions/
│   ├── expressive/
│   └── functional/
├── orion_motion/
│   ├── motion_loader.py
│   ├── motion_validator.py
│   ├── trajectory_builder.py
│   ├── ros_motion_player.py
│   └── ros_pose_player.py
├── test/
├── package.xml
└── setup.py
```

Each file has one main responsibility:

- `motion_loader.py` reads YAML safely. It does not decide whether the data is
  meaningful.
- `motion_validator.py` enforces the motion-data contract, including all five
  joints, finite numbers, known pose names, and mechanical position limits.
- `trajectory_builder.py` converts symbolic pose names and relative durations
  into ordered numeric targets and absolute timestamps.
- `ros_motion_player.py` converts a resolved trajectory into ROS 2 messages and
  sends a `FollowJointTrajectory` action goal.
- `ros_pose_player.py` converts a direct named-pose request into the same
  one-keyframe trajectory path.
- `simulation/mujoco/motion_player.py` executes the same resolved trajectory by
  stepping the native MuJoCo model.

Keeping these responsibilities separate makes errors easier to locate. A YAML
syntax failure belongs to loading; an unknown pose belongs to validation; a
wrong controller endpoint belongs to the ROS adapter.

## Pose Representation

A pose is a complete named destination:

```yaml
attentive:
  description: Forward-facing posture with a focused, curious head tilt.
  positions:
    base_yaw_joint: -0.30
    shoulder_pitch_joint: -0.10
    elbow_pitch_joint: -0.28
    head_roll_joint: -0.65
    head_pitch_joint: -0.22
```

Every pose must contain exactly the same five semantic joint names. Partial
poses are intentionally rejected because silently retaining unspecified joint
values would make a motion depend on hidden simulator state.

The order written in a pose mapping is not used as the actuator order. Array
consumers use the authoritative `joint_order` in `motion_limits.yaml`:

```text
base_yaw_joint
shoulder_pitch_joint
elbow_pitch_joint
head_roll_joint
head_pitch_joint
```

That explicit ordering prevents a numeric position from being sent to the
wrong actuator.

The configured limits are mechanical URDF limits, not evidence-based operating
or stability limits. Tighter safety limits belong to later milestones.

## Motion Representation

A motion describes timing between named poses:

```yaml
format_version: 1

motion:
  name: acknowledge_expressive
  description: Pause, nod once, and settle back into the attentive pose.
  keyframes:
    - pose: attentive
      duration: 0.25
      hold: 0.10
    - pose: acknowledge_nod_down
      duration: 0.25
      hold: 0.12
    - pose: acknowledge_nod_rebound
      duration: 0.20
      hold: 0.06
    - pose: attentive
      duration: 0.30
      hold: 0.45
```

For each keyframe:

- `duration` is the travel time from the preceding state or keyframe.
- `hold` is the stationary time after arriving at the keyframe.
- Both are seconds.
- `duration` must be positive; `hold` may be zero but not negative.

The builder accumulates absolute timing. For example:

```text
first arrival = 0.00 + 0.25 = 0.25
first hold end = 0.25 + 0.10 = 0.35
second arrival = 0.35 + 0.25 = 0.60
```

The resolved result contains joint names, complete numeric positions, arrival
times, hold-end times, and total duration. It still contains no simulator API.

## Why Holds Become Duplicate ROS Points

A ROS trajectory point states where each joint should be at one timestamp. To
represent a hold, the ROS adapter sends the same positions twice:

```text
t = 0.25: arrive at nod-down pose
t = 0.37: remain at nod-down pose
```

Without the second point, the trajectory controller could immediately begin
interpolating toward the next keyframe. The duplicate target gives the pause an
explicit end time.

## Direct Named-Pose Requests

The command:

```bash
ros2 run orion_motion go_to_pose attentive --duration 1.5
```

does not bypass the motion system. `build_pose_trajectory()` creates an
in-memory one-keyframe motion and sends it through the same validation,
canonical joint ordering, ROS conversion, and action client used by stored
motions.

This is preferable to a second pose-specific controller path. One destination
and a multi-keyframe animation therefore have identical execution semantics.

Use `--dry-run` to inspect the exact controller goal without contacting ROS:

```bash
ros2 run orion_motion go_to_pose attentive --duration 1.5 --dry-run
```

## Gazebo Execution

Gazebo exposes Orion's five position interfaces through `gz_ros2_control`. The
`joint_trajectory_controller` owns those interfaces and exposes:

```text
/joint_trajectory_controller/follow_joint_trajectory
```

The motion player waits for that action server, sends the complete trajectory,
waits for acceptance, and then waits for the controller result. A successful
Python process therefore means the controller reported success, not merely that
a message was published.

Typical workflow:

```bash
cd /home/mofe/Desktop/dev/orion/ros2_ws
source /opt/ros/jazzy/setup.bash
colcon build --packages-select orion_motion
source install/setup.bash

ros2 launch orion_description gazebo.launch.py
```

In another terminal with the same two setup files sourced:

```bash
ros2 run orion_motion go_to_pose attentive
ros2 run orion_motion play_motion acknowledge_expressive
```

The workspace must be rebuilt after adding a console command or changing
installed YAML. Otherwise ROS may execute an older copy from `install/`.

## MuJoCo Execution

The native MuJoCo player loads the package data from the source tree, resolves
the same trajectory, maps semantic joint names to MuJoCo joint and actuator
IDs, and advances the simulator one physics step at a time.

```bash
cd /home/mofe/Desktop/dev/orion
.venv/bin/python simulation/mujoco/motion_player.py \
  acknowledge_expressive \
  --start-pose attentive
```

For automated completion and final-error checks:

```bash
.venv/bin/python simulation/mujoco/motion_player.py \
  acknowledge_expressive \
  --start-pose attentive \
  --lead-in 0 \
  --headless
```

MuJoCo currently uses its native stepping adapter rather than
`mujoco_ros2_control`. Introducing the shared ROS controller stack for MuJoCo is
part of Milestone 3 simulator parity. The motion data does not need to change
when that adapter changes.

## First Functional and Expressive Pairs

All Milestone 2 A/B demonstrations use `attentive` as their controlled start
pose.

| Intent | Functional | Expressive | Final pose |
|---|---|---|---|
| Look at predefined left target | Directly turn left | Anticipate, lean, overshoot, settle | `look_left` |
| Acknowledge user | Remain attentive | Pause, nod, rebound, settle | `attentive` |
| Target unreachable | Remain attentive after rejection | Pause, attempt, face user, shake no, settle | `attentive` |

The functional acknowledgement and unreachable demonstrations are stationary
only because they start at `attentive`. Arbitrary measured-state no-op handling
and start-state continuity belong to Milestone 3.

The target-unreachable motion communicates a predefined failure scenario. The
motion package does not calculate reachability. Actual 3D target solving and an
explicit unreachable result belong to Milestone 4.

## Reproducing the A/B Test

Before each variant, reset to the controlled starting pose:

```bash
ros2 run orion_motion go_to_pose attentive --duration 1.0 --hold 0.3
```

Then compare:

```bash
ros2 run orion_motion play_motion look_at_left
ros2 run orion_motion play_motion look_at_left_expressive

ros2 run orion_motion play_motion acknowledge
ros2 run orion_motion play_motion acknowledge_expressive

ros2 run orion_motion play_motion target_unreachable
ros2 run orion_motion play_motion target_unreachable_expressive
```

Reset to `attentive` between every command. Otherwise the second animation has
a different initial condition and the comparison is not controlled.

## Tests

Run the pure package suite:

```bash
cd /home/mofe/Desktop/dev/orion/ros2_ws/src/orion_motion
source /opt/ros/jazzy/setup.bash
python3 -m pytest -q
```

Run through the ROS build system:

```bash
cd /home/mofe/Desktop/dev/orion/ros2_ws
source /opt/ros/jazzy/setup.bash
colcon build --packages-select orion_motion
colcon test --packages-select orion_motion --event-handlers console_direct+
colcon test-result --verbose
```

The tests cover safe loading, schema validation, joint limits, canonical joint
ordering, accumulated timing, hold conversion, installed pose lookup, unknown
pose rejection, and required final poses.

## Adding a Pose or Motion Yourself

To add a pose:

1. Add a descriptive name to `config/poses.yaml`.
2. Include all five canonical joints in radians.
3. Keep every value within `motion_limits.yaml`.
4. Run the tests.
5. Inspect with `go_to_pose NAME --dry-run`.
6. Validate visually in a simulator.

To add a motion:

1. Decide its functional intent and intended starting pose.
2. Reuse existing named poses where their meaning matches.
3. Add new poses only for genuinely distinct keyframes.
4. Create a YAML file under `motions/functional` or `motions/expressive`.
5. Use pose names, travel durations, and holds rather than raw joint arrays.
6. Add a test for important sequence or final-pose invariants.
7. Rebuild, dry-run, and test in both simulators from the same start pose.

## Deliberate Milestone 2 Limits

Milestone 2 proves representation, validation, and basic execution. It does not
yet provide:

- Native MuJoCo ROS control integration.
- Measured-start continuity guarantees.
- Velocity, acceleration, or jerk-aware generation.
- Cancellation or preemption.
- Stability and collision checks.
- Task-space target solving.
- A looping scene runtime.
- Orion Studio.

Those omissions are roadmap boundaries, not hidden claims that the current
motion system already handles them.
