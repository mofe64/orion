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


- **Requested**: what motion we want.
- **Generated**: the exact path, speed, and acceleration we want.
- **Validated**: the generated path passed every configured check.
- **Executed**: a backend tried to follow the validated path.
- **Measured**: what the simulated or real robot actually did.

Keeping these stages separate matters. A valid motion file is not enough. Its
path may still move too fast, cross a forbidden area, or begin far away from
the robot's real position.

## Where Motion Data Lives

The motion system is split into small files so each part has one job:

```text
ros2_ws/src/orion_motion/
├── config/
│   ├── poses.yaml
│   ├── motion_limits.yaml
│   ├── forbidden_regions.yaml
│   ├── execution_policy.yaml
│   └── stability_limits.yaml
├── motions/
│   ├── functional/
│   └── expressive/
└── orion_motion/
    ├── motion_loader.py
    ├── motion_validator.py
    ├── trajectory_builder.py
    ├── trajectory_generator.py
    ├── trajectory_validator.py
    ├── execution_types.py
    ├── ros_state_reader.py
    ├── ros_motion_player.py
    └── ros_pose_player.py

simulation/mujoco/
├── motion_player.py
├── mujoco_backend.py
└── stability_monitor.py
```

The configuration and motion files mean:

- `poses.yaml` stores reusable named joint positions.
- `motions/functional/` stores direct motions for a task.
- `motions/expressive/` stores motions that communicate with more character.
- `motion_limits.yaml` defines joint order and movement limits.
- `forbidden_regions.yaml` defines unsafe combinations of joint positions.
- `execution_policy.yaml` defines ROS feedback, tolerance, and timeout rules.
- `stability_limits.yaml` defines native MuJoCo settling and base-stability
  checks.

The Python files have separate responsibilities:

- `motion_loader.py` reads YAML safely. It does not decide whether the content
  makes sense.
- `motion_validator.py` checks the pose and motion file structure, names,
  values, and basic position limits.
- `trajectory_builder.py` replaces pose names with complete joint positions
  and changes relative durations into absolute times.
- `trajectory_generator.py` combines that request with measured starting state
  and creates the exact smooth path.
- `trajectory_validator.py` checks the complete generated path. It is the only
  file that can create a `ValidatedTrajectory`.
- `execution_types.py` defines the feedback, measurements, and final result
  records shared by the backends.
- `ros_state_reader.py` reads fresh joint position and velocity by name from
  `/joint_states`.
- `ros_motion_player.py` sends a validated motion through
  `FollowJointTrajectory`.
- `ros_pose_player.py` turns one named-pose command into the same motion path.
- `mujoco_backend.py` maps semantic names to MuJoCo joints and actuators.
- `simulation/mujoco/motion_player.py` samples the generated path at every
  MuJoCo physics step.
- `stability_monitor.py` measures base movement, tilt, height, and floor
  contact.

Keeping these jobs separate makes problems easier to locate.

## Poses and Motions

A pose is one complete named joint configuration:

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

Every pose contains all five joints. Partial poses are rejected because an
omitted joint would otherwise depend on hidden simulator or robot state.

The order written in the YAML mapping is not used as array order. Code uses the
canonical `joint_order` from `motion_limits.yaml`:

```text
base_yaw_joint
shoulder_pitch_joint
elbow_pitch_joint
head_roll_joint
head_pitch_joint
```

This prevents a numeric value from being sent to the wrong joint.

A motion gives timing to named poses:

```yaml
format_version: 1

motion:
  name: look_at_left
  description: Turn toward a predefined target on Orion's left.
  keyframes:
    - pose: look_left
      duration: 1.5
      hold: 0.5
```

- `duration` is the travel time from the preceding state to this pose.
- `hold` is the time spent stationary after reaching the pose.
- Both values are seconds.
- Duration must be greater than zero. Hold may be zero but not negative.

The builder changes relative timing into absolute timing. For example:

```text
first arrival = 0.00 + 0.25 = 0.25
first hold end = 0.25 + 0.10 = 0.35
next arrival = 0.35 + 0.25 = 0.60
```

The built request contains complete numeric joint targets and their arrival and
hold-end times. It still contains no ROS or MuJoCo object.

Functional and expressive motions use the same pipeline. The folder only
describes the motion's purpose; it does not change the safety rules.

A direct pose command also uses this pipeline:

```bash
ros2 run orion_motion go_to_pose attentive --duration 1.5
```

`ros_pose_player.py` creates a one-keyframe motion in memory. It does not bypass
loading, validation, measured state, trajectory generation, or ROS execution.
One destination and a stored animation therefore follow the same rules.

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

Every move between two poses uses a quintic curve:

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

For one transition, Orion uses this time-scaling curve:

```text
u = t / T
s(u) = 10u^3 - 15u^4 + 6u^5
q(t) = q0 + (q1 - q0)s(u)
```

- `q0` is the starting joint position.
- `q1` is the destination joint position.
- `T` is the transition duration.
- `t` is the elapsed time inside the transition.

You do not need to calculate this by hand to author a motion. The important
idea is that the curve starts and ends with zero speed and acceleration.

For a joint displacement `D = |q1 - q0|`, shorter time produces larger peaks:

```text
peak velocity     = 1.875 D / T
peak acceleration = (10 / sqrt(3)) D / T^2
peak jerk         = 60 D / T^3
```

This explains why cutting a duration in half is a large change. Velocity
doubles, acceleration becomes four times larger, and jerk becomes eight times
larger.

A generated trajectory contains:

- the five canonical joint names;
- position, velocity, and acceleration at every boundary point;
- quintic transition segments;
- constant-position hold segments;
- calculated peak dynamics for every joint and transition;
- total duration.

For a one-pose motion with a hold, the points look like:

```text
t = 0.00  measured start, stopped
t = 1.50  arrive at the named pose, stopped
t = 2.00  finish holding the same pose, stopped
```

## Check the Complete Trajectory

Only the validator can create a `ValidatedTrajectory`. Both execution backends
require that type, so an unchecked generated trajectory cannot be played by
mistake.

`motion_limits.yaml` keeps different kinds of limits separate:

- mechanical position limits describe the joint's full modelled range;
- operational position limits describe the range normal motions may use;
- velocity, acceleration, and jerk limits control how demanding a transition
  may be;
- cancellation deceleration limits control how quickly a cancelled joint may
  slow down.

The simulation values are development limits. They are not measured safety
limits for the physical servos or lamp structure.

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

Generation and validation answer different questions:

```text
GeneratedTrajectory = the exact requested path
ValidationReport    = every problem found in that path
ValidatedTrajectory = a path that passed every configured check
```

Keeping the raw generated path is useful even when it is unsafe. Orion can show
which joint and segment failed, the measured peak, the allowed limit, and the
minimum required duration. Both execution backends reject the raw type and
accept only `ValidatedTrajectory`.

Forbidden regions can constrain one joint or several joints at the same time.
The validator checks the continuous line through joint space rather than a few
sampled points. This catches a crossing even if both endpoints lie outside the
forbidden region.

`forbidden_regions.yaml` currently contains an explicit empty list because no
evidence-backed self-collision regions have been defined. This does **not**
mean every joint combination is physically safe.

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

The controller saying "success" is not enough by itself. Orion then reads
fresh joint states and checks that the final position and low velocity remain
inside their limits for 0.25 seconds. If they do not, the result is
`settling_failed`.

Each saved feedback sample contains:

- joint names;
- desired position, velocity, and acceleration;
- measured position, velocity, and acceleration;
- the error between desired and measured state;
- trajectory time and feedback time.

These records do not import ROS or MuJoCo. Each backend translates its own
measurements into the same plain records, which makes comparisons possible.

Orion also checks that the measured starting state is still fresh just before
the goal is sent. A state can become old while waiting for the action server.

The result wait has a finite wall-clock deadline. Gazebo can run slower than
real time, so the deadline is longer than the trajectory's simulated time. If
the deadline expires, Orion cancels the goal and returns a timeout.

The deadline is calculated as:

```text
trajectory duration × timeout factor
    + controller goal-time tolerance
    + wall-clock margin
```

This keeps the wait finite while allowing a simulator to run slower than real
time.

Cancellation has three parts:

1. Orion sends one cancellation request, even if cancellation is requested
   more than once.
2. The trajectory controller slows every joint using its configured maximum
   cancellation deceleration.
3. Orion waits until fresh joint feedback remains below the stopped-speed
   limit for a short time.

Orion only marks the stop as confirmed after the third part succeeds. Closing
the command with Ctrl+C uses this same path. The result also records how long
stopping took and how far each joint moved after cancellation was requested.

If a new motion replaces an active motion, only the newest waiting request is
kept. Orion stops the old motion, reads fresh joint state, and generates the
replacement from that measured stopped position. It never continues from
where the old command expected the robot to be.

The controller and client use matching simulation tolerances:

```text
path position error: 0.20 rad
final position error: 0.05 rad
stopped velocity:     0.05 rad/s
extra goal time:      0.50 s
```

These are explicit so a tracking failure cannot be treated as success.

When the backend label is `gazebo`, the same result also records movement of
`base_footprint` and contact between Orion's support box and the floor. Native
MuJoCo measures the physical base and contact directly from MuJoCo physics.

## Run Directly in MuJoCo

The native MuJoCo player receives the same `ValidatedTrajectory`. At each
physics step it samples the shared curve and sends the desired positions to
MuJoCo's actuators.

This route is useful because it is small, fast, and independent of ROS. It is
good for checking the model and running headless tests.

It does not use a ROS action server or `joint_trajectory_controller`. It still
creates the same kind of `ExecutionResult` and records desired, measured, and
error values at every physics step.

Reaching the end time is not success by itself. The final position and
velocity must remain inside their limits for the full settling time. The
player also measures:

- maximum tracking error for every joint;
- base translation;
- base tilt;
- base height change;
- how long the base loses floor contact.

A limit failure returns an unsafe-stability result. Closing the viewer returns
cancelled, not success. The optional `--report-json` argument saves the full
result and feedback in a machine-readable file.

## ROS-Controlled MuJoCo

Orion can run MuJoCo behind `mujoco_ros2_control`:

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

Gazebo and MuJoCo therefore have the same ROS controller interface. Start one
simulator, then use the same `ros2 run orion_motion play_motion ...` command.

The native runner is still useful for fast model and stability tests. It writes
targets directly to MuJoCo and does not use ROS. ROS-controlled MuJoCo adds a
second route; it does not replace the native runner.

## Adding a Pose or Motion

To add a pose:

1. Add a descriptive name under `poses` in `config/poses.yaml`.
2. Include all five canonical joint names in radians.
3. Keep every position inside the configured limits.
4. Run `go_to_pose NAME --dry-run` to inspect the generated command.
5. Test the pose slowly in simulation.

To add a motion:

1. Decide what the motion should communicate or accomplish.
2. Reuse existing poses when they already express the required shape.
3. Add new poses only for genuinely different keyframes.
4. Create one YAML file under `motions/functional` or `motions/expressive`.
5. Use named poses, transition durations, and holds.
6. Run a dry run and read every validation error.
7. Test the same motion from a known stopped state in both simulators.

Do not weaken a limit just to make one motion pass. If the motion is too fast,
increase its authored duration or redesign its keyframes.

## Useful Checks

Run the ROS package tests:

```bash
cd /home/mofe/Desktop/dev/orion/ros2_ws
source /opt/ros/jazzy/setup.bash
colcon test --packages-select orion_motion --event-handlers console_direct+
```

Run MuJoCo through ROS control:

```bash
cd /home/mofe/Desktop/dev/orion
source /opt/ros/jazzy/setup.bash
source ros2_ws/install/setup.bash
ros2 launch orion_description mujoco.launch.py
```

In another terminal, source the same setup files and play a motion:

```bash
ros2 run orion_motion play_motion look_at_left
```

To save the desired and measured ROS controller data, identify the simulator
behind the controller and choose a JSON file:

```bash
ros2 run orion_motion play_motion look_at_left \
  --backend-label gazebo \
  --report-json /tmp/look-at-left-gazebo.json
```

Run the same motion in the other simulator, using the label
`mujoco_ros2_control`, then compare the two files:

```bash
ros2 run orion_motion compare_motion_runs \
  /tmp/look-at-left-gazebo.json \
  /tmp/look-at-left-mujoco.json
```

The comparison checks that both runs used the same motion file, limits, joint
order, target poses, and timing. It also shows each simulator's tracking and
final errors.

To repeat a cancellation at a known point in controller time:

```bash
ros2 run orion_motion play_motion return_home \
  --cancel-at 0.5 \
  --backend-label gazebo \
  --report-json /tmp/orion-cancel.json
```

To replace an active motion with a newer request:

```bash
ros2 run orion_motion play_motion look_at_left \
  --replace-with return_home \
  --replace-at 0.5 \
  --backend-label gazebo \
  --report-json /tmp/orion-replace.json
```

Cancellation succeeds only after a measured stop is confirmed. Replacement
succeeds only when the first result is `preempted`, its stop is confirmed,
and the newer motion finishes successfully.

Run native MuJoCo headlessly:

```bash
cd /home/mofe/Desktop/dev/orion
.venv/bin/python simulation/mujoco/motion_player.py \
  look_at_left --start-pose attentive --headless \
  --report-json /tmp/orion-look-left.json
```

The ROS control layer is explained in
[How ROS Controls Orion](orion_ros_control.md). The MuJoCo model itself is
explained in [Orion MuJoCo Model Basics](orion_mujoco_model_basics.md).
