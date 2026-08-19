# How ROS Controls Orion

ROS control connects Orion's movement code to a robot backend. Today that
backend can be Gazebo. The same connection will also be used for MuJoCo through
`mujoco_ros2_control`.

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

- `controller_manager` loads controllers and runs them repeatedly.
- `joint_state_broadcaster` publishes what the joints are actually doing.
- `joint_trajectory_controller` moves the joints along a timed path.

## Controller Manager

The controller manager runs a loop. Each loop does this:

1. Read the joint state from the backend.
2. Update the active controllers.
3. Calculate new commands.
4. Send the commands back to the backend.

In Gazebo, the `gz_ros2_control` plugin creates the controller manager. Orion's
launch file does not create a second one.

Its settings are in:

```text
ros2_ws/src/orion_description/config/orion_controllers.yaml
```

The controller-manager name in this file must match the real ROS node name. If
the names differ, ROS will not apply the settings.

## Reading Joint State

The joint-state broadcaster publishes `/joint_states`. A message contains
measured values such as:

- position: where a joint is now;
- velocity: how fast it is moving;
- effort: how strongly the backend says it is being driven.

The broadcaster only reports state. It does not move the robot.

## Sending a Trajectory

The trajectory controller accepts a `FollowJointTrajectory` goal. A goal
contains:

- joint names;
- desired positions;
- the time for each point;
- desired velocities and accelerations.

Orion sends all five moving joints:

```text
base_yaw_joint
shoulder_pitch_joint
elbow_pitch_joint
head_roll_joint
head_pitch_joint
```

Partial goals are disabled. Even when only the head should move, the goal must
include safe targets for the other joints.

The controller owns the position-command interface for these joints while it
is active. This stops another controller from commanding the same interface at
the same time.

## Command Is Not Measurement

A position command means "please move here." A position state means "this is
where the joint is now."

They are different while the robot is moving. Orion keeps controller feedback
so it can compare the desired and measured motion.

## Why Names Must Match

The same joint names must appear in:

1. the URDF;
2. the URDF's `<ros2_control>` section;
3. `orion_controllers.yaml`;
4. every trajectory goal.

A spelling difference is not harmless. ROS treats it as a different joint.

## Limits and Tolerances

Joint limits answer "is this command allowed?" Tracking tolerances answer "is
the robot following the command closely enough?"

Orion uses both:

- command limits keep requested positions inside the configured range;
- path tolerance checks error during movement;
- goal tolerance checks the final position;
- stopped-velocity tolerance checks that the joints have settled;
- goal-time tolerance gives the controller a small amount of extra time.

These checks are explained as part of the full path in
[How Orion Moves](orion_motion_system.md).
