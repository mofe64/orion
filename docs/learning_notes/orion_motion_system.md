# How Orion Moves

This note follows one motion from its name to the robot. ROS and native MuJoCo
share the same motion data and the same checked desired path.

## The Whole Path

```text
motion name
    |
    v
motion file + named poses
    |
    v
requested keyframes and timing
    |
    + measured starting joint state
    + motion limits
    v
smooth generated trajectory
    |
    + safety checks
    v
validated trajectory
    |
    +---------------------+
    |                     |
    v                     v
ROS controller       native MuJoCo
```

The words have simple meanings:

- **Requested**: what motion we want.
- **Generated**: the exact path, speed, and acceleration we want.
- **Validated**: the generated path passed every configured check.
- **Executed**: a backend tried to follow the validated path.
- **Measured**: what the simulated or real robot actually did.

Keeping these stages separate matters. A valid motion file is not enough. Its
path may still move too fast, cross a forbidden area, or begin far away from
the robot's real position.

## Where Motion Data Lives

The motion package contains these files:

```text
config/poses.yaml              reusable named poses
motions/functional/            direct task motions
motions/expressive/            motions with more character
config/motion_limits.yaml      position and movement limits
config/forbidden_regions.yaml  unsafe joint combinations
config/execution_policy.yaml   ROS feedback and timeout rules
```

A pose gives one position for every moving joint. A motion refers to poses and
adds transition and hold times. This means several motions can reuse the same
pose without copying all five joint values.

Functional and expressive motions use the same pipeline. The folder only
describes the motion's purpose; it does not change the safety rules.

## Start From What the Robot Is Doing

Before live ROS playback, Orion reads `/joint_states`. It needs a position and
velocity for every moving joint.

The reading must be:

- complete;
- made of finite numbers;
- recent enough;
- inside the allowed position range;
- slow enough to count as stopped.

Incoming joint order does not matter because Orion matches values by joint
name. Orion then puts them back into its one canonical order.

The first trajectory point is the measured position at time zero. This avoids
a jump from an assumed pose to the robot's real pose.

A dry run has no live robot state, so it needs an explicit preview pose:

```bash
ros2 run orion_motion play_motion look_at_left \
  --dry-run \
  --start-pose attentive
```

The named start pose is only for preview. Normal ROS execution uses measured
state.

## Make the Path Smooth

Every move between two poses uses a quintic curve. The name is less important
than its behaviour:

- it starts at the first pose;
- it ends at the next pose;
- its speed is zero at both ends;
- its acceleration is zero at both ends;
- its position, speed, and acceleration change smoothly.

A hold is a constant position with zero speed and acceleration.

The transition duration controls how demanding the move is. Moving the same
distance in less time needs more speed, acceleration, and jerk. Jerk means how
quickly acceleration changes.

The curve has finite jerk, and Orion checks its peak jerk. It is not a fully
jerk-limited hardware motion planner.

## Check the Complete Trajectory

Only the validator can create a `ValidatedTrajectory`. Both execution backends
require that type, so an unchecked generated trajectory cannot be played by
mistake.

The validator checks:

- all joints and values are present;
- times increase correctly;
- positions stay inside operational limits;
- peak velocity is allowed;
- peak acceleration is allowed;
- peak jerk is allowed;
- the continuous path does not enter a configured forbidden region.

It checks the path between points, not only the endpoints. Two safe endpoints
can still have an unsafe path between them.

Validation reports every problem it finds. It does not stop after the first
one, and it does not silently make a motion slower. When a duration is too
short, the report includes the minimum duration needed for the failed limit.

`forbidden_regions.yaml` currently contains an explicit empty list because no
evidence-backed self-collision regions have been defined. This does **not**
mean every joint combination is physically safe.

The limits are development values for simulation. They are not proven limits
for Orion's physical motors or lamp structure.

## Run Through ROS

The ROS adapter sends the validated trajectory to
`joint_trajectory_controller`. Every point includes desired position,
velocity, and acceleration.

The controller sends feedback while it runs. Orion keeps:

- desired joint state;
- measured joint state;
- the error between them;
- trajectory time and feedback time.

The final `ExecutionResult` says whether the goal succeeded, was rejected,
violated a path or goal tolerance, timed out, or failed for another reason.
This is more useful than a simple true-or-false answer.

Orion also checks that the measured starting state is still fresh just before
the goal is sent. A state can become old while waiting for the action server.

The result wait has a finite wall-clock deadline. Gazebo can run slower than
real time, so the deadline is longer than the trajectory's simulated time. If
the deadline expires, Orion requests cancellation and returns a timeout.
Requesting cancellation does not yet prove that the robot has stopped.

The controller and client use matching simulation tolerances:

```text
path position error: 0.20 rad
final position error: 0.05 rad
stopped velocity:     0.05 rad/s
extra goal time:      0.50 s
```

These are explicit so a tracking failure cannot be treated as success.

## Run Directly in MuJoCo

The native MuJoCo player receives the same `ValidatedTrajectory`. At each
physics step it samples the shared curve and sends the desired positions to
MuJoCo's actuators.

This route is useful because it is small, fast, and independent of ROS. It is
good for checking the model and running headless tests.

It does not use a ROS action server or `joint_trajectory_controller`. Its
completion check is also simpler: it reports final joint error but does not
yet use the same feedback, settling, cancellation, and result records as the
ROS route.

## ROS-Controlled MuJoCo

Native MuJoCo is not meant to be Orion's only MuJoCo route. Orion will also use
`mujoco_ros2_control`:

```text
same Orion ROS motion player
          |
same FollowJointTrajectory goal
          |
same joint_trajectory_controller
          |
mujoco_ros2_control
          |
MuJoCo physics
```

That gives Gazebo and MuJoCo the same ROS controller interface. It lets us
compare the simulators without changing the motion client or controller rules.

The sensible build order is:

1. Make ROS cancellation stop safely and confirm that the joints stopped.
2. Add settling and stability measurements to the native MuJoCo runner.
3. Connect Orion's MuJoCo model to `mujoco_ros2_control`.
4. Run the same ROS motion client against Gazebo and MuJoCo and compare them.

So ROS-controlled MuJoCo starts after the cancellation work and native
stability measurements. It is the next simulator-integration step after
those two foundations. The native runner stays because it remains valuable
for fast model tests; ROS-controlled MuJoCo adds a second route rather than
replacing it.

## Useful Checks

Run the ROS package tests:

```bash
cd /home/mofe/Desktop/dev/orion/ros2_ws
source /opt/ros/jazzy/setup.bash
colcon test --packages-select orion_motion --event-handlers console_direct+
```

Run native MuJoCo headlessly:

```bash
cd /home/mofe/Desktop/dev/orion
.venv/bin/python simulation/mujoco/motion_player.py \
  look_at_left --start-pose attentive --headless
```

The ROS control layer is explained in
[How ROS Controls Orion](orion_ros_control.md). The MuJoCo model itself is
explained in [Orion MuJoCo Model Basics](orion_mujoco_model_basics.md).
