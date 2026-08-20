# How ROS Controls Orion

ROS control connects Orion's motion code to the robot that should move. The
robot can be simulated in Gazebo or MuJoCo. Later, the same control structure
can connect to Orion's real motors.

The controller settings are stored in:

```text
ros2_ws/src/orion_description/config/orion_controllers.yaml
```

This file says which controllers should exist and how they should behave.

## The Main Pieces

```text
Orion motion player
        |
        | FollowJointTrajectory goal
        v
joint_trajectory_controller
        |
        | position commands
        v
robot backend
        |
        | measured joint state
        v
joint_state_broadcaster
        |
        v
/joint_states
```

There are three important parts:

- `controller_manager` loads and runs controllers.
- `joint_trajectory_controller` moves Orion's joints along a timed path.
- `joint_state_broadcaster` publishes what the joints are actually doing.

## The Controller Manager

`controller_manager` is responsible for:

- loading controller plugins;
- configuring controllers;
- activating and deactivating controllers;
- running the controller update loop;
- managing access to hardware interfaces;
- preventing two controllers from commanding the same interface;
- reading joint state from the robot backend;
- sending controller commands to the robot backend.

The first part of Orion's controller file looks like this:

```yaml
controller_manager:
  ros__parameters:
    update_rate: 100
    enforce_command_limits: true

    joint_state_broadcaster:
      type: joint_state_broadcaster/JointStateBroadcaster

    joint_trajectory_controller:
      type: joint_trajectory_controller/JointTrajectoryController
```

The name `controller_manager` must match the real controller-manager node name.
If the names differ, ROS will not apply these settings to the node.

`ros__parameters` contains the values passed to that node.

### `update_rate: 100`

This asks the controller manager to run its control loop 100 times per second.
One loop does four things:

1. Read joint state from the backend.
2. Update the active controllers.
3. Calculate new commands.
4. Write the commands back to the backend.

The simulator also has a physics loop. The physics loop and controller loop are
related, but they are not the same thing. The simulator calculates gravity,
contacts, and movement. The controller calculates what joint command should be
sent next.

### `enforce_command_limits: true`

This tells the controller manager to enforce the joint limits from Orion's
robot description.

For example, if a position command is outside a joint's allowed range, a
saturation limiter keeps the command at the nearest valid boundary before it
is sent to the backend.

This protection does not prove that a whole motion is safe. Orion also checks
its operational position, velocity, acceleration, and jerk limits before
sending a trajectory.

## The Joint-State Broadcaster

This declaration creates a controller named `joint_state_broadcaster`:

```yaml
joint_state_broadcaster:
  type: joint_state_broadcaster/JointStateBroadcaster
```

The name on the left is the name of this controller instance. The value after
`type` tells the controller manager which compiled controller plugin to load.

The broadcaster reads state interfaces from the backend. Examples include:

```text
base_yaw_joint/position
base_yaw_joint/velocity
base_yaw_joint/effort
```

It publishes the measurements on `/joint_states`.

These are measured values from Gazebo, MuJoCo, or the real robot backend. They
are not simply copies of the requested targets. The broadcaster does not move
anything; it only reports state.

## The Joint-Trajectory Controller

This declaration creates the controller responsible for motion:

```yaml
joint_trajectory_controller:
  type: joint_trajectory_controller/JointTrajectoryController
```

The controller accepts a `FollowJointTrajectory` goal. A goal contains:

1. Joint names.
2. Desired joint positions.
3. Times when those positions should be reached.
4. Desired velocities and accelerations.

The controller calculates intermediate commands between the supplied
trajectory points.

For example:

```text
start position:  base yaw = 0.00 rad
goal position:   base yaw = 0.10 rad
arrival time:    3.00 seconds
```

The controller does not immediately jump from `0.00` to `0.10`. It sends
intermediate position commands during those three seconds.

Orion supplies position, velocity, and acceleration at every trajectory point.
The controller uses those values to make a smooth quintic path between points.

## Which Joints the Controller Owns

The controller's full configuration has a `joints` list:

```yaml
joint_trajectory_controller:
  ros__parameters:
    joints:
      - base_yaw_joint
      - shoulder_pitch_joint
      - elbow_pitch_joint
      - head_roll_joint
      - head_pitch_joint
```

When the controller becomes active, it claims the position-command interface
for all five joints. Claiming an interface means the controller has exclusive
permission to write commands to it. Another controller cannot command that
same interface at the same time.

The joint names must match exactly in:

1. The URDF joint definitions.
2. The URDF `<ros2_control>` section.
3. `orion_controllers.yaml`.
4. Every trajectory goal.

A spelling difference creates a different name as far as ROS is concerned.

## Command Interfaces

This setting tells the trajectory controller what kind of command it may send:

```yaml
command_interfaces:
  - position
```

Orion currently uses position commands. A position command means, "move this
joint toward this angle."

The URDF exposes a position-command interface for every moving joint:

```xml
<joint name="base_yaw_joint">
  <command_interface name="position">
    <param name="min">-5.02103</param>
    <param name="max">1.26215</param>
  </command_interface>

  <state_interface name="position"/>
  <state_interface name="velocity"/>
  <state_interface name="effort"/>
</joint>
```

Other robots may use velocity commands, such as "rotate at 0.2 radians per
second," or effort commands, such as "apply this amount of torque." Orion's
current controller does not use those command types.

## State Interfaces

The trajectory controller reads these measurements:

```yaml
state_interfaces:
  - position
  - velocity
```

- Position state tells the controller where a joint is now.
- Velocity state tells the controller how fast the joint is moving.

The controller uses these measurements to check whether Orion is following the
trajectory and whether the final goal has been reached.

The URDF also exposes effort state. The joint-state broadcaster can publish it,
but the trajectory controller does not need it for Orion's position-command
configuration.

## Command Is Not Measurement

A position command and a position state are different:

```text
position command = where we want the joint to go
position state   = where the joint is now
```

They are usually different while the robot is moving. The backend may lag,
overshoot, or be unable to reach the target. This is why Orion records desired
and measured motion separately.

## Why Every Goal Contains Five Joints

Orion has this setting:

```yaml
allow_partial_joints_goal: false
```

It means every trajectory goal must include all five controlled joints. A goal
containing only this would be rejected:

```yaml
joint_names:
  - base_yaw_joint
```

The complete list is:

```yaml
joint_names:
  - base_yaw_joint
  - shoulder_pitch_joint
  - elbow_pitch_joint
  - head_roll_joint
  - head_pitch_joint
```

If a joint should not move during a motion, its trajectory points should keep
the correct held position for that joint. We must not blindly use zero because
zero may be different from its current position and would make it move.

## Reaching the End of a Goal

Orion also uses:

```yaml
allow_nonzero_velocity_at_trajectory_end: false
```

The final trajectory point must ask for zero velocity. This helps make the
motion end at rest instead of passing through the final pose while still
moving.

`action_monitor_rate: 20.0` makes the controller check the action connection
20 times per second.

## Limits and Tolerances

Joint limits answer, "is this command allowed?" Tolerances answer, "is the
robot following the command closely enough?"

Orion uses:

- path tolerance to limit tracking error while moving;
- goal tolerance to limit final position error;
- stopped-velocity tolerance to decide whether a joint has stopped;
- goal-time tolerance to allow a small amount of extra settling time.

For one joint, the configuration looks like this:

```yaml
base_yaw_joint:
  trajectory: 0.20
  goal: 0.05
  max_deceleration_on_cancel: 4.0
```

- `trajectory: 0.20` allows at most `0.20 rad` of path error.
- `goal: 0.05` allows at most `0.05 rad` of final position error.
- `max_deceleration_on_cancel: 4.0` limits how quickly cancellation slows the
  joint.

The same fields exist for all five joints.

## Cancellation

When a goal is cancelled, the controller uses:

```yaml
constraints:
  decelerate_on_cancel: true
```

The controller slows the joints using each joint's configured maximum
cancellation deceleration. It does not immediately replace the moving command
with a fixed hold target.

After the controller accepts cancellation, Orion reads fresh `/joint_states`
messages. It only confirms the stop after all five measured velocities remain
below the stopped-velocity limit for a short time.

If a new motion replaces an active motion, Orion:

1. Cancels the active goal.
2. Confirms that the joints stopped.
3. Reads the new measured positions.
4. Generates the replacement trajectory from those positions.
5. Sends the new goal.

This prevents the replacement from starting at a position where the old
motion expected the robot to be.

## Gazebo and MuJoCo

In Gazebo, the `gz_ros2_control` plugin creates the controller manager. Orion's
Gazebo launch file does not create another controller manager.

In MuJoCo, the `mujoco_ros2_control` node creates the controller manager and
runs the MuJoCo simulation.

The backend changes, but these parts stay the same:

- controller names;
- five joint names;
- position and state interfaces;
- controller configuration;
- `FollowJointTrajectory` action;
- Orion motion player.

This is why the same motion command works with either simulator.

## Running Orion

Install the MuJoCo ROS adapter once:

```bash
sudo apt install ros-jazzy-mujoco-ros2-control
```

Start Gazebo:

```bash
ros2 launch orion_description gazebo.launch.py
```

Or start MuJoCo:

```bash
ros2 launch orion_description mujoco.launch.py
```

To run MuJoCo without its window:

```bash
ros2 launch orion_description mujoco.launch.py headless:=true
```

Only run one simulator at a time because both provide a controller manager with
the same controller names.

After the selected simulator is ready, play a motion:

```bash
ros2 run orion_motion play_motion look_at_left
```

The motion player talks to `joint_trajectory_controller`. It does not need to
know whether Gazebo or MuJoCo is behind the controller.

The complete motion path is explained in
[How Orion Moves](orion_motion_system.md).
