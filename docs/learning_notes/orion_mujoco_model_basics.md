# Orion MuJoCo Model Basics

This note explains the main concepts used in Orion's MuJoCo `robot.xml` file. It also explains the terms used to describe Orion's simulated servo motors.

## What MJCF Is

MJCF stands for MuJoCo XML configuration format. It is MuJoCo's native format for describing a simulated model.

Our ROS URDF and MuJoCo MJCF describe the same physical robot, but they serve different systems:

```text
URDF = Orion's main ROS robot description
MJCF = Orion's native MuJoCo simulation description
```

MJCF can directly describe MuJoCo-specific features such as:

- Actuators
- Sensors
- Joint damping and friction
- Motor force limits
- Simulation defaults
- Contact geometry

## How `robot.xml` Was Created

The comment at the top of `robot.xml` says:

```xml
<!-- Generated using onshape-to-robot -->
```

This tells us that the original model was generated from Onshape CAD using `onshape-to-robot`.

The generated parts include:

- The body hierarchy
- Mesh positions and rotations
- Joint positions, axes, and ranges
- Masses
- Centres of mass
- Inertia values

The file also contains sections marked:

```xml
<!-- Additional sensors.xml -->
```

and:

```xml
<!-- Additional joints_properties.xml -->
```

This suggests that sensor and servo settings were added to the generated model from separate configuration files.

The complicated CAD-derived values should not normally be typed or changed by hand without a specific reason. Simulation settings such as actuators, sensors, and damping are more likely to be deliberately maintained by a person.

## Compiler Settings

The model contains:

```xml
<compiler angle="radian" meshdir="assets" autolimits="true"/>
```

### `angle="radian"`

Joint angles and ranges are interpreted as radians.

For example:

```text
0.5 rad is approximately 28.6 degrees
```

### `meshdir="assets"`

Mesh filenames are resolved relative to the `assets` directory next to `robot.xml`.

For example:

```xml
<mesh file="lamphead.stl"/>
```

refers to:

```text
simulation/mujoco/assets/lamphead.stl
```

### `autolimits="true"`

MuJoCo automatically treats a joint or actuator as limited when it has a range.

This saves us from having to repeat a separate `limited="true"` setting everywhere a range is provided.

## Defaults and Classes

MuJoCo defaults let us define a reusable set of properties once and apply it to many elements.

For example:

```xml
<default class="sts3215">
  <joint damping="0.60" frictionloss="0.052" armature="0.028"/>
  <position kp="17.8" kv="0.0" forcerange="-3.35 3.35"/>
</default>
```

Any joint or position actuator using:

```xml
class="sts3215"
```

receives those settings.

This keeps the model consistent and avoids copying the same servo settings onto all five joints.

The root body also contains:

```xml
childclass="simulation"
```

This makes the `simulation` defaults apply to its child elements unless they select a more specific class.

Defining a class does not automatically create a body, joint, or actuator. An element must use the class before those settings affect the simulation.

## The Body Hierarchy

MuJoCo represents the robot as nested bodies:

```xml
<body name="parent_body">
  <body name="child_body">
  </body>
</body>
```

A child body's position and rotation are defined relative to its parent body.

The nesting creates the kinematic tree, in the same way that parent and child links create the tree in URDF.

### `pos`

The `pos` value gives an XYZ translation relative to the parent body:

```xml
pos="0.0058 0.0183 0.0829"
```

The values are measured in metres.

### `quat`

The `quat` value gives the body's rotation as a quaternion:

```xml
quat="0.570913 -0.417203 0.570913 0.417203"
```

MuJoCo writes quaternions in this order:

```text
w x y z
```

The generated MuJoCo model is rooted at `lamparm__base_elbow`, rather than at the physical base. This is the same unusual CAD-generated root structure that we previously found in the original URDF.

We are keeping it unchanged until the original MuJoCo model has been run and validated.

## The Free Joint

The root body contains:

```xml
<freejoint name="lamparm__base_elbow_freejoint"/>
```

A free joint allows the complete robot to:

- Move along X, Y, and Z
- Rotate around X, Y, and Z

This gives the robot six free-body degrees of freedom and makes Orion free-standing.

The free joint is not one of Orion's five servo joints, and it is not connected to an actuator.

If the free joint were removed, the root body would be fixed to the MuJoCo world.

## Hinge Joints

Orion's five servo joints use:

```xml
type="hinge"
```

A hinge joint rotates around one axis. It is MuJoCo's equivalent of a revolute joint in URDF.

For example:

```xml
<joint
  axis="0 0 1"
  name="shoulder_pitch_joint"
  type="hinge"
  range="-1.0842575 2.0573351"
  class="sts3215"/>
```

### Joint axis

```xml
axis="0 0 1"
```

means that the joint rotates around the local Z axis of its body.

All five generated joints use the same local axis, but their bodies have different rotations. This means their axes can point in different directions in the world.

### Joint range

```xml
range="-1.0842575 2.0573351"
```

defines the joint's minimum and maximum angles in radians.

These ranges match the limits preserved in Orion's URDF.

## Semantic Joint Names

The generated MuJoCo model originally used numeric joint names. After validating that each actuator moved the expected part, we renamed the joints to match Orion's semantic ROS names:

| Original name | Current name | Physical role |
|---|---|---|
| `1` | `base_yaw_joint` | Rotates the lamp above the base |
| `2` | `shoulder_pitch_joint` | Moves the lower arm |
| `3` | `elbow_pitch_joint` | Moves the upper arm |
| `4` | `head_roll_joint` | Rolls the lamp head assembly |
| `5` | `head_pitch_joint` | Tilts the lamp head |

MuJoCo and ROS now use the same joint names. This reduces the risk of commanding the wrong joint and makes it easier for a future control layer to support both simulators.

## Inertial Properties

Each moving body has an inertial element similar to:

```xml
<inertial
  pos="0.0729066 -0.0229022 0.217607"
  mass="0.234138"
  fullinertia="0.0010919 0.00129476 0.000320305 7.35843e-05 -0.000504692 0.000161616"/>
```

### `mass`

`mass` is the total mass of that body in kilograms.

### Inertial `pos`

The inertial `pos` is the position of the body's centre of mass relative to the body frame.

### `fullinertia`

`fullinertia` describes how the mass is distributed and how difficult the body is to rotate.

The six values are:

```text
Ixx Iyy Izz Ixy Ixz Iyz
```

These values came from the CAD model and should not normally be guessed by hand.

## Visual and Collision Geometry

Most parts appear twice in `robot.xml`:

```xml
<geom type="mesh" class="visual" .../>
<geom type="mesh" class="collision" .../>
```

The visual geometry is what we see in the viewer.

The collision geometry is what MuJoCo uses when calculating contact with the floor and other objects.

### Disabling collisions for visual geometry

The visual class contains:

```xml
<geom type="mesh" contype="0" conaffinity="0" group="2"/>
```

Setting `contype` and `conaffinity` to zero prevents the visual copy from participating in collision detection.

Without this separation, the same part could be represented twice in contact calculations.

Orion currently uses detailed meshes for both visual and collision geometry. This preserves the original shape, but detailed mesh collisions can require more computation than simple collision shapes.

## Mesh Assets and Materials

Meshes are registered in the asset section:

```xml
<asset>
  <mesh file="lamphead.stl"/>
</asset>
```

Once registered, a geometry can refer to the mesh by name:

```xml
<geom mesh="lamphead" .../>
```

Materials define colours using RGBA values:

```xml
<material
  name="lamphead_material"
  rgba="0.301961 0.301961 0.301961 1"/>
```

RGBA means:

```text
red green blue alpha
```

Alpha controls transparency, where `1` means fully opaque.

The MuJoCo model still references the original Chinese servo-horn filenames. These files must keep matching the names in `robot.xml` unless both the XML references and mesh filenames are renamed together.

## Position Actuators

We can think of a position actuator as a simulated servo motor. We give it a target angle, and it applies turning force to move the joint towards that target.

The shared actuator settings are:

```xml
<position
  kp="17.8"
  kv="0.0"
  forcerange="-3.35 3.35"/>
```

The actuator section creates one position actuator for each servo joint:

```xml
<position
  class="sts3215"
  name="shoulder_pitch_joint"
  joint="shoulder_pitch_joint"
  inheritrange="1"/>
```

This means:

- Use the shared STS3215 settings.
- Name the actuator `shoulder_pitch_joint`.
- Connect it to the joint named `shoulder_pitch_joint`.
- Use the joint's range as the actuator's allowed target range.

An actuator does not teleport the joint to its target. It applies limited torque while MuJoCo continues to calculate mass, gravity, friction, damping, and contact.

## Position Error

Position error is the difference between the requested angle and the joint's current angle.

```text
Position error = requested position - current position
```

For example:

```text
Requested angle: 0.5 rad
Current angle:   0.4 rad
Position error:  0.1 rad
```

The actuator sees that the joint is 0.1 rad away from its target and applies torque to reduce that error.

## `kp` — Position Strength

```xml
kp="17.8"
```

`kp` controls how strongly the actuator tries to move the joint towards its requested position.

We can think of it like a spring pulling the joint towards the target:

- A higher `kp` creates a stronger and more aggressive response.
- A lower `kp` creates a weaker and softer response.
- If `kp` is too high, the joint may shake or become unstable.
- If `kp` is too low, the joint may struggle to move or hold the arm against gravity.

The simplified idea is:

```text
Corrective torque = kp × position error
```

For example:

```text
kp:                17.8
Position error:     0.1 rad
Calculated torque:  1.78 N·m
```

The actual torque is still restricted by the actuator's force range.

## `kv` — Speed-Based Slowing

```xml
kv="0.0"
```

`kv` makes the actuator resist the joint based on how fast the joint is moving. This can help slow the joint as it approaches its target and reduce bouncing.

Our value is zero, so the position actuator does not add this extra speed-based slowing force.

Orion's joints still have joint damping, so the movement is not completely undamped.

```text
kp      = pulls the joint towards its target
kv      = can slow the joint based on its speed
damping = also resists joint movement
```

## `forcerange` — Maximum Motor Strength

```xml
forcerange="-3.35 3.35"
```

`forcerange` limits how much turning force, or torque, the actuator can apply.

For Orion's hinge joints:

```text
Maximum torque in one direction: +3.35 N·m
Maximum torque in the other:     -3.35 N·m
```

Even if the joint is far from its target, the simulated servo cannot generate unlimited torque.

For example:

```text
Calculated torque: 8.00 N·m
Maximum allowed:    3.35 N·m
Applied torque:     3.35 N·m
```

This makes the simulated actuator behave more like a real motor with limited strength.

## Joint Damping

Orion's simulated servo joints contain:

```xml
<joint damping="0.60"/>
```

Damping resists movement when a joint is rotating. Faster movement produces more resistance.

It helps stop the arm from swinging or bouncing for too long. It is similar to the resistance provided by a door closer or a shock absorber.

## `frictionloss` — Joint Friction

```xml
frictionloss="0.052"
```

`frictionloss` represents dry friction inside the joint. It creates resistance that opposes movement.

Unlike damping, which increases with speed, friction loss represents resistance that does not depend directly on how fast the joint is moving.

```text
damping      = resistance related to speed
frictionloss = dry friction resisting movement
```

## `armature` — Reflected Motor Inertia

```xml
armature="0.028"
```

`armature` adds inertia from the motor and gearbox to the joint.

It represents the fact that the servo's internal rotating parts also resist changes in motion. A larger value makes it harder to accelerate or decelerate the joint quickly.

It does not add visible mass or geometry. It changes the joint's dynamic behaviour.

## Backlash

Backlash is a small amount of looseness between gears.

Imagine turning a loose door handle. The handle may move slightly before the mechanism inside begins to move. A geared servo can have a similar small gap between its gear teeth.

The MuJoCo model defines this reusable setting:

```xml
<default class="backlash">
  <joint
    limited="true"
    range="-0.0087266 0.0087266"/>
</default>
```

The range is approximately -0.5° to +0.5°. It represents a small amount of possible movement before the gears fully engage.

However, defining a class does not automatically use it. A joint would need to reference it with something like:

```xml
<joint class="backlash"/>
```

None of Orion's current MuJoCo joints use the `backlash` class. The setting exists in the file, but backlash is not currently being simulated.

## Sensors and Sites

The model defines an accelerometer and gyroscope:

```xml
<sensor>
  <accelerometer name="accel_sensor" site="imu_site"/>
  <gyro name="gyro_sensor" site="imu_site"/>
</sensor>
```

Both sensors measure motion at:

```xml
<site name="imu_site" .../>
```

A site is a named position and orientation attached to a body. It is similar to a coordinate frame or measurement point.

The site does not need its own mass because it is not a separate physical body. It follows the body to which it is attached.

## Actuator Declaration Order

After the semantic rename, the actuators are declared in Orion's standard order:

```text
base_yaw_joint
shoulder_pitch_joint
elbow_pitch_joint
head_roll_joint
head_pitch_joint
```

MuJoCo's control array follows actuator declaration order:

| Control index | Actuator and joint |
|---:|---|
| `ctrl[0]` | `base_yaw_joint` |
| `ctrl[1]` | `shoulder_pitch_joint` |
| `ctrl[2]` | `elbow_pitch_joint` |
| `ctrl[3]` | `head_roll_joint` |
| `ctrl[4]` | `head_pitch_joint` |

Even though the order is now clear, future control code should still look up actuators by name instead of assuming that an index will never change.

## Summary

```text
MJCF           = MuJoCo's native model format
body           = a rigid part in the kinematic tree
freejoint      = lets the complete robot move freely in the world
hinge joint    = rotates around one axis
inertial       = mass, centre of mass, and rotational inertia
visual geom    = geometry shown in the viewer
collision geom = geometry used for contact calculations
site           = a massless measurement point attached to a body
actuator       = applies force or torque to control a joint
position error = difference between the target and current angle
kp             = how strongly the servo moves towards the target
kv             = extra slowing based on joint speed
forcerange     = maximum motor torque
damping        = speed-related resistance that reduces swinging
frictionloss   = dry friction that resists movement
armature       = simulated inertia from the motor and gearbox
backlash       = small gear looseness, currently defined but unused
```

The central idea is that the position actuator provides a target, but MuJoCo still decides the actual movement by simulating the actuator limits, joint properties, mass, gravity, and collisions.
