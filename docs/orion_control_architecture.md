# Orion Control Architecture

## Status of This Document

This document records Orion's shared control architecture and the remaining bring-up work.

Orion currently has:

- A semantic ROS robot description
- RViz visualization
- A working Gazebo `ros2_control` integration
- A MuJoCo `ros2_control` integration using the validated native model
- A compiled and unit-tested physical STS3215 `SystemInterface`
- Backend-specific Gazebo, MuJoCo, and physical launch files

The physical adapter has not yet been exercised against the assembled lamp.
One unified `backend:=...` bring-up command and hardware diagnostics/shutdown
orchestration remain future work.

## Architecture Decision

Orion should have one shared control interface and one shared controller stack.

It should not have one backend implementation containing Gazebo, MuJoCo, and physical-servo code. Each system needs its own adapter because each one communicates differently.

```text
                       Orion behaviours
                              |
                       Motion planner/player
                              |
                 FollowJointTrajectory action
                              |
                 joint_trajectory_controller
                              |
                    controller_manager
                              |
                  ros2_control interfaces
                              |
             +----------------+----------------+
             |                |                |
             v                v                v
      Gazebo adapter     MuJoCo adapter    STS3215 adapter
             |                |                |
             v                v                v
      Gazebo physics     MuJoCo physics    Physical servos
```

The shared `ros2_control` boundary is what gives Orion one control architecture.

The adapters are replaceable implementations behind that boundary.

## What "One Backend" Should Mean for Orion

From the user's point of view, Orion should have one bringup command with a backend choice:

```text
backend = gazebo
backend = mujoco
backend = hardware
```

A future command might look like:

```bash
ros2 launch orion_bringup orion.launch.py backend:=gazebo
```

or:

```bash
ros2 launch orion_bringup orion.launch.py backend:=mujoco
```

or:

```bash
ros2 launch orion_bringup orion.launch.py backend:=hardware
```

These are proposed commands. The `orion_bringup` package does not exist yet.

Internally, the selected backend would load a different adapter. This gives us one workflow without mixing three unrelated implementations into one plugin.

## What Remains Shared

The following parts should remain the same across all backends:

- Semantic joint names
- Joint order where arrays are required
- Position command interfaces
- Position and velocity state interfaces
- Joint limits
- Controller names
- Trajectory message format
- `FollowJointTrajectory` action interface
- Higher-level motion and behaviour software

Orion's shared joint contract is:

```text
base_yaw_joint
shoulder_pitch_joint
elbow_pitch_joint
head_roll_joint
head_pitch_joint
```

The normal high-level command should remain:

```text
/joint_trajectory_controller/follow_joint_trajectory
```

The software sending that goal should not need to know which backend is active.

## What Changes Between Backends

Each adapter converts the shared interfaces into backend-specific operations.

| Backend | Command destination | State source |
|---|---|---|
| Gazebo | Gazebo simulated joints | Gazebo joint state |
| MuJoCo | MuJoCo actuators | MuJoCo `qpos`, `qvel`, and sensor data |
| Hardware | STS3215 servo-bus commands | Physical servo feedback |

Other backend-specific differences include:

- How simulation or wall time is provided
- How the system is launched
- How sensors are connected
- How errors and disconnections are handled
- Which external libraries are required

## The Shared Control Cycle

The controller manager runs the common control cycle:

```text
1. Read state from the selected adapter
2. Update active controllers
3. Produce new command values
4. Write commands through the selected adapter
```

For Orion, the joint trajectory controller produces intermediate position setpoints over time. The selected adapter receives those setpoints through position command interfaces.

### Command path

```text
Trajectory goal
    -> joint_trajectory_controller
    -> position command interfaces
    -> selected backend adapter
    -> simulator actuator or physical servo
```

### Feedback path

```text
Simulator or servo measurement
    -> selected backend adapter
    -> position and velocity state interfaces
    -> joint_state_broadcaster
    -> /joint_states
```

## Gazebo Adapter

The existing Gazebo setup uses:

```xml
<plugin>gz_ros2_control/GazeboSimSystem</plugin>
```

This adapter converts `ros2_control` commands into Gazebo joint commands and converts Gazebo state into ROS state interfaces.

Gazebo remains responsible for:

- Gravity
- Floor contact
- Collision
- Mass and inertia
- Free-standing base movement

The free base must remain unactuated and must not be attached to the world.

## MuJoCo Adapter

The preferred future MuJoCo adapter is the existing ROS Controls integration:

```xml
<plugin>mujoco_ros2_control/MujocoSystemInterface</plugin>
```

This adapter is intended to expose MuJoCo through the same `ros2_control` system-interface boundary.

Its conceptual work is:

```text
read()
    MuJoCo state
        -> ros2_control state interfaces

write()
    ros2_control position commands
        -> MuJoCo actuator targets
```

Orion should continue using the validated native MJCF files:

```text
simulation/mujoco/scene.xml
simulation/mujoco/robot.xml
```

The five semantic MuJoCo actuator names now match the ROS joint names, which prepares the model for this mapping.

MuJoCo remains responsible for:

- Gravity
- Floor contact
- Collision
- Mass and inertia
- Actuator dynamics
- Free-standing base movement

## The Role of `mujoco_vendor`

`mujoco_vendor` packages the native MuJoCo C/C++ library and headers so ROS packages can depend on MuJoCo through the ROS build and dependency system.

It can support the MuJoCo adapter, but it is not a universal backend.

```text
mujoco_vendor
    -> supplies MuJoCo library to mujoco_ros2_control
    -> does not control Gazebo
    -> does not communicate with STS3215 servos
```

The project-local Python environment has a different role:

```text
.venv
    -> native MuJoCo viewer
    -> MJCF compile checks
    -> future Python learning and test scripts
```

The Python environment should remain useful even after ROS–MuJoCo integration exists because it lets us validate the native model independently of ROS.

## Physical STS3215 Adapter

Physical Orion uses its own `ros2_control` system interface:

```xml
<plugin>orion_hardware/STS3215System</plugin>
```

The adapter is responsible for:

- Opening and closing the servo bus
- Mapping radians to servo units
- Writing target positions
- Reading measured positions and velocities
- Reporting communication failures
- Enforcing safe startup and shutdown behaviour
- Avoiding sudden motion when controllers activate

The trajectory controller should not contain serial-port or servo-protocol code. That belongs behind the hardware adapter.

The implementation lives in `ros2_ws/src/orion_hardware`. It uses the
software calibration JSON as the joint-to-servo mapping, applies the selected
LeRobot/LeLamp PID and acceleration profile while torque is off, seeds each
goal register from measured position before torque-on, and uses synchronized
state reads and position-only writes. It deliberately does not impose a
`Goal_Velocity` or `Torque_Limit` ceiling.

`ros2 launch orion_description hardware.launch.py` selects this adapter while
preserving the same five position command interfaces and trajectory action.
Successful build/fake-transport tests are not evidence of physical direction,
tracking, thermal, or shutdown validation; those remain commissioning work.

## Free-Standing Base and Sensors

Only the five servo joints should receive position commands.

The free-standing base should be handled by physics in Gazebo and MuJoCo and by the real world on physical hardware.

```text
Five servo joints -> commanded
Free base         -> observed, not commanded
```

A future simulator integration may publish the free body's world pose through a separate state publisher. Publishing the pose must not create a constraint that fixes the base.

The MuJoCo IMU currently provides accelerometer and gyroscope data at `imu_site`. A later sensor integration can expose that data through ROS sensor state interfaces and an IMU broadcaster.

## Future Package Boundaries

A possible future package layout is:

```text
ros2_ws/src/
├── orion_description/   Robot description, meshes, and frames
├── orion_control/       Shared controllers and joint contract
├── orion_bringup/       One launch entry point and backend selection
├── orion_gazebo/        Gazebo-specific world and launch support
├── orion_mujoco/        MuJoCo ros2_control configuration and launch support
└── orion_hardware/      Physical STS3215 system interface
```

This layout is a future direction. Packages should be created only when their responsibilities are needed.

## Future Backend-Selection Workflow

### Step 1 — define the shared contract

Keep one authoritative list of:

- Joint names
- Joint limits
- Command interfaces
- State interfaces
- Units

Use radians, radians per second, and N·m consistently.

### Step 2 — separate shared control configuration

Move backend-neutral controller configuration into a shared control package when the second ROS backend is introduced.

The common controller configuration should include:

- `joint_state_broadcaster`
- `joint_trajectory_controller`
- The five semantic joint names
- Position command interfaces
- Position and velocity state interfaces

Backend-specific timing or tuning may still require small configuration overlays.

### Step 3 — make the hardware plugin selectable

The current URDF directly selects the Gazebo plugin. When MuJoCo integration begins, use a controlled description mechanism, likely Xacro, to select exactly one system interface:

```text
gazebo  -> gz_ros2_control/GazeboSimSystem
mujoco  -> mujoco_ros2_control/MujocoSystemInterface
hardware -> orion_hardware/STS3215System
```

Only one adapter should own the five command interfaces at a time.

### Step 4 — create one bringup entry point

Create a launch argument named `backend` and validate its value.

The bringup layer should start:

- The selected simulator or hardware connection
- `robot_state_publisher`
- Controller manager where required
- Shared controllers
- The correct time source

### Step 5 — integrate MuJoCo first

Before physical hardware, install and test `mujoco_ros2_control` with a small demonstration model.

Then connect Orion's validated `scene.xml` and verify:

- All five semantic actuators are found
- Position commands reach the correct actuators
- Position and velocity states return correctly
- `/joint_states` is published
- The free joint remains unactuated
- Simulation time advances correctly
- IMU values can be accessed

### Step 6 — add the physical adapter

Implement the STS3215 system interface only after the shared control contract is stable.

Begin with read-only servo discovery and state reporting before enabling movement.

Add command output only after startup position handling, limits, and communication-failure behaviour are understood.

### Step 7 — run backend parity tests

Send the same slow trajectory to each backend:

```text
base_yaw_joint       =  0.10 rad
shoulder_pitch_joint =  0.15 rad
elbow_pitch_joint    = -0.10 rad
head_roll_joint      =  0.05 rad
head_pitch_joint     =  0.10 rad
```

Verify that:

- The same joint names are accepted
- Each joint moves in the same semantic direction
- Limits are respected
- Final state is reported through the same ROS interfaces
- Failure information reaches the caller

The physical motion does not need to look perfectly identical across all backends. Different physics and motor models will produce different transient behaviour. The shared contract and meaning of each command must remain identical.

## Important Invariants

The future architecture must preserve these rules:

1. Higher-level behaviour code never imports Gazebo, MuJoCo, or servo-driver APIs.
2. Joint and actuator mappings use semantic names rather than assumed array indices.
3. Exactly one backend owns each position command interface.
4. Exactly one component owns the MuJoCo stepping loop.
5. The free-standing base is never silently fixed to the world.
6. Simulation nodes use the correct simulation clock.
7. Physical hardware uses safe startup, shutdown, and communication-failure behaviour.
8. Backend-specific code stays behind the `ros2_control` system interface.

## Decision Summary

Orion's target is one control experience, not one universal backend implementation.

```text
One launch interface
One trajectory interface
One shared controller stack
One semantic joint contract
Multiple replaceable backend adapters
```

For MuJoCo, the preferred future path is to evaluate `mujoco_ros2_control` using `mujoco_vendor` while preserving the native MJCF model and the independent Python validation workflow.

For physical Orion, a dedicated STS3215 adapter should implement the same command and state interfaces.

This structure lets behaviours and motion software remain unchanged while the selected adapter determines where the command is executed.

## References

- [ros2_control architecture](https://control.ros.org/jazzy/doc/getting_started/getting_started.html)
- [ROS 2 joint trajectory controller](https://control.ros.org/jazzy/doc/ros2_controllers/joint_trajectory_controller/doc/userdoc.html)
- [mujoco_ros2_control repository](https://github.com/ros-controls/mujoco_ros2_control)
- [MuJoCo ROS 2 system-interface API](https://control.ros.org/jazzy/doc/api/classmujoco__ros2__control_1_1MujocoSystemInterface.html)
