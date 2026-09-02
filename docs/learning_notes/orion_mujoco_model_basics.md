# How Orion's MuJoCo model works

[`simulation/mujoco/robot.xml`](../../simulation/mujoco/robot.xml) defines the
maintained MuJoCo model. Physical limits and calibration remain separate
runtime inputs.

MuJoCo needs a model that describes Orion's bodies, joints, mass, shapes,
motors, sensors, and surroundings. MuJoCo's XML format is called MJCF.

Orion's Unified Robot Description Format (URDF) and MJCF describe the same
robot for different systems:

```text
URDF = the neutral kinematic and geometry description
MJCF = the robot and physics description used by MuJoCo
```

MJCF can directly describe MuJoCo features such as actuators, contact shapes,
joint friction, sensors, and simulation settings.

## The Two XML Files

Orion separates the robot from its surroundings:

```text
simulation/mujoco/robot.xml  robot bodies, joints, motors, and sensors
simulation/mujoco/scene.xml  floor, light, camera settings, and physics step
```

`scene.xml` includes `robot.xml`:

```xml
<include file="robot.xml"/>
```

This means the robot model can be placed in another scene without copying its
complete body definition.

## Generated Geometry and Maintained Settings

Most body positions, mesh transforms, masses, and inertia values came from the
CAD model through `onshape-to-robot`. These values describe the shape and mass
of the real mechanism and should not be changed by guessing.

The file also contains settings that we deliberately maintain, including:

- joint damping and friction;
- position actuators;
- actuator force limits;
- the free-standing base contact shape;
- sensors and sites.

It is useful to separate these two kinds of information:

```text
CAD-derived values       = where parts are and how mass is distributed
simulation settings      = how MuJoCo should model motors, contact, and motion
```

## Compiler Settings

At the top of `robot.xml`:

```xml
<compiler angle="radian" meshdir="../../description/meshes" autolimits="true"/>
```

### `angle="radian"`

Joint angles and ranges are measured in radians.

```text
0.5 rad is about 28.6 degrees
```

The same unit is used by Orion's shared motion files.

### `meshdir="../../description/meshes"`

Mesh paths are resolved from Orion's shared `description/meshes` directory.

For example:

```xml
<mesh file="lamphead.stl"/>
```

refers to:

```text
description/meshes/lamphead.stl
```

If a mesh file is renamed, its XML reference must also be renamed.

### `autolimits="true"`

When a joint or actuator has a range, MuJoCo automatically treats it as
limited. We do not need to repeat `limited="true"` on every element.

## Defaults and Classes

MuJoCo defaults let several elements share the same settings.

Orion's simulated servo class is:

```xml
<default class="sts3215">
  <joint damping="0.60" frictionloss="0.052" armature="0.028"/>
  <position kp="17.8" kv="0.0" forcerange="-3.35 3.35"/>
</default>
```

A joint or actuator that uses:

```xml
class="sts3215"
```

receives these shared settings. This keeps all five simulated servos
consistent and avoids copying the same values many times.

Defining a class does not create a joint or actuator. An element must use the
class before the settings have any effect.

## Bodies Form a Tree

A MuJoCo `<body>` is similar to a URDF link. Bodies are nested to form a tree:

```xml
<body name="parent_body">
  <body name="child_body">
  </body>
</body>
```

The child body's position and rotation are relative to its parent. When a joint
moves, its child body and every body below that child move with it.

### `pos`

`pos` is an XYZ translation from the parent body, measured in metres:

```xml
pos="0.0058 0.0183 0.0829"
```

### `quat`

`quat` gives orientation as a quaternion:

```xml
quat="0.570913 -0.417203 0.570913 0.417203"
```

MuJoCo writes quaternion values in this order:

```text
w x y z
```

Quaternions are another way to describe rotation. The generated values are
hard to read by eye, but they avoid some problems that Euler-angle rotations
can have.

## The Free Root

The root body contains:

```xml
<freejoint name="lamparm__base_elbow_freejoint"/>
```

A free joint gives the complete robot six ways to move:

- translation along X, Y, and Z;
- rotation around X, Y, and Z.

This makes Orion free-standing. Gravity and floor contact can move the complete
robot. The free joint is not one of Orion's five servo joints and has no
actuator.

If the free joint were removed, the root body would be fixed to the MuJoCo
world. That would hide tipping and sliding instead of simulating them.

## The Physical Base and Floor Contact

The generated body tree begins at the upper assembly rather than the physical
base. The physical base is lower in the tree. This unusual structure matters
when code places Orion in a starting pose.

Changing the five joint positions can move the base body as a side effect.
`mujoco_backend.py` corrects the free-root transform after setting a pose so the
physical base remains at the same world position and orientation.

The base uses a simple box for floor contact:

```xml
<geom name="base_floor_collision" type="box" .../>
```

The detailed meshes still provide the visible shape. The simple box provides a
flat, stable contact surface and is cheaper for physics than a complicated
triangle mesh.

## Hinge Joints

Orion's five servo joints use `type="hinge"`. A hinge rotates around one axis,
like a revolute joint in URDF.

For example:

```xml
<joint
  axis="0 0 1"
  name="shoulder_pitch_joint"
  type="hinge"
  range="-1.0842575 2.0573351"
  class="sts3215"/>
```

- `axis="0 0 1"` means rotation around the body's local Z axis.
- `range` gives the minimum and maximum angle in radians.
- `class="sts3215"` applies the shared simulated-servo settings.

The bodies have different orientations, so the same local joint axis can point
in a different world direction for each hinge.

MuJoCo, the native runtime, and the motion library use the same five names:

```text
base_yaw_joint
shoulder_pitch_joint
elbow_pitch_joint
head_roll_joint
head_pitch_joint
```

Their physical locations and jobs are explained in
[Orion's Joint Structure](orion_joints.md).

## Mass and Inertia

Each moving body has an inertial section similar to:

```xml
<inertial
  pos="0.0729066 -0.0229022 0.217607"
  mass="0.234138"
  fullinertia="0.0010919 0.00129476 0.000320305 7.35843e-05 -0.000504692 0.000161616"/>
```

### `mass`

`mass` is the body's mass in kilograms.

### Inertial `pos`

The inertial position is the body's centre of mass relative to its body frame.
Gravity acts as though the body's mass is concentrated around this balance
point.

### `fullinertia`

`fullinertia` describes how the mass is distributed and how difficult the body
is to rotate:

```text
Ixx Iyy Izz Ixy Ixz Iyz
```

Two objects with the same mass can rotate differently when their mass is
distributed differently. These values come from the CAD model and should not
be guessed.

Incorrect mass or inertia can make Orion fall, shake, or accelerate
unrealistically even when the visible model looks correct.

## Visual and Collision Geometry

Many parts appear twice:

```xml
<geom type="mesh" class="visual" .../>
<geom type="mesh" class="collision" .../>
```

- Visual geometry is drawn in the viewer.
- Collision geometry is used for contacts and physics.

The visual class contains:

```xml
<geom type="mesh" contype="0" conaffinity="0" group="2"/>
```

Setting `contype` and `conaffinity` to zero keeps the visual copy out of
collision detection. Otherwise the same part could be counted twice during
contact calculations.

Meshes are registered in the asset section:

```xml
<asset>
  <mesh file="lamphead.stl"/>
</asset>
```

A geometry then refers to the registered mesh by name:

```xml
<geom mesh="lamphead" .../>
```

Materials define colour with red, green, blue, and alpha values. An alpha value
of `1` is fully opaque.

## Position Actuators

A position actuator behaves like a simulated servo motor. We give it a target
angle, and it applies limited torque to move the joint toward that angle.

One actuator is declared for every controlled joint:

```xml
<position
  class="sts3215"
  name="shoulder_pitch_joint"
  joint="shoulder_pitch_joint"
  inheritrange="1"/>
```

This means:

- use the shared `sts3215` actuator settings;
- name the actuator `shoulder_pitch_joint`;
- connect it to the joint with the same name;
- use the joint range as the allowed target range.

An actuator does not teleport a joint. MuJoCo still calculates mass, gravity,
friction, damping, contact, and motor limits.

## Position Error

Position error is the difference between the target and measured angle:

```text
position error = target position - measured position
```

For example:

```text
target angle:    0.50 rad
measured angle:  0.40 rad
position error:  0.10 rad
```

The actuator applies torque to reduce this error.

## `kp`: Position Strength

Orion uses:

```xml
kp="17.8"
```

`kp` controls how strongly position error produces corrective torque. A useful
simplified model is:

```text
corrective torque = kp × position error
```

For a `0.10 rad` error:

```text
17.8 × 0.10 = 1.78 N·m
```

- Higher `kp` gives a stronger, more aggressive response.
- Lower `kp` gives a softer, weaker response.
- Too high can cause shaking or overshoot.
- Too low can make the arm struggle against gravity.

The actual torque is still restricted by `forcerange`.

## `kv`: Speed-Based Slowing

Orion uses:

```xml
kv="0.0"
```

`kv` makes the actuator resist motion according to joint speed. This can slow a
joint as it approaches the target and reduce bouncing.

Orion's value is zero, so the actuator adds no extra speed-based slowing. The
joints still have damping, which also resists movement.

```text
kp      = pulls toward the target
kv      = actuator resistance related to speed
damping = joint resistance related to speed
```

## `forcerange`: Maximum Motor Strength

Orion uses:

```xml
forcerange="-3.35 3.35"
```

For a hinge actuator, this limits torque to `3.35 N·m` in either direction.

For example:

```text
calculated torque:  8.00 N·m
maximum allowed:    3.35 N·m
applied torque:     3.35 N·m
```

The actuator cannot create unlimited torque just because the target is far
away.

These values are simulation settings. They are not proof that the model
exactly matches a physical STS3215 servo.

## Passive Joint Behaviour

Joint properties affect movement even when the target does not change.

### `damping`

```xml
damping="0.60"
```

Damping resists motion in proportion to joint speed. It helps stop the arm
from swinging or bouncing for too long, like a door closer.

### `frictionloss`

```xml
frictionloss="0.052"
```

Friction loss represents dry joint friction. It opposes movement without being
directly proportional to speed.

```text
damping      = resistance that increases with speed
frictionloss = dry resistance opposing movement
```

### `armature`

```xml
armature="0.028"
```

Armature adds reflected inertia from the motor and gearbox. It makes quick
acceleration and deceleration harder without adding visible geometry.

These three settings strongly affect overshoot, settling, and stability. They
should be changed with a test and a reason, not only because one animation
looks better.

## Backlash

Backlash is a small amount of looseness between gear teeth. A joint may move
slightly before the gears fully engage.

The model defines a reusable backlash class:

```xml
<default class="backlash">
  <joint
    limited="true"
    range="-0.0087266 0.0087266"/>
</default>
```

This range is about `-0.5°` to `+0.5°`.

None of Orion's joints use this class. The definition exists, but the model
does not simulate backlash unless a joint explicitly references the class.

## Sensors and Sites

A site is a named position and orientation attached to a body. It is a
measurement point, not a separate physical part, so it needs no mass.

Orion has an `imu_site` with two MuJoCo sensors:

```xml
<sensor>
  <accelerometer name="accel_sensor" site="imu_site"/>
  <gyro name="gyro_sensor" site="imu_site"/>
</sensor>
```

As the body moves, the site moves with it. The accelerometer and gyroscope
therefore measure motion at the IMU's location.

The model includes these MuJoCo IMU sensors, but `RuntimeDriver` does not expose
them through Orion's native status interface.

## Joint and Actuator Lookup

Each position actuator has the same semantic name as its joint. Native MuJoCo
uses these names to connect commands to the correct joint.

The actuator declarations happen to follow this order:

```text
base_yaw_joint
shoulder_pitch_joint
elbow_pitch_joint
head_roll_joint
head_pitch_joint
```

Native code still looks up joint and actuator IDs by name. It does not assume
that XML array index zero will always mean base yaw. This prevents a reordered
XML file from sending a command to the wrong joint.

## Native MuJoCo control

Orion drives the MJCF model through its native adapter:

```text
shared motion YAML
    -> Rust trajectory compiler
    -> native MuJoCo player
    -> MuJoCo position actuators
```

The adapter keeps the free root unactuated and uses measured simulated state
for completion and stability checks.

The shared movement path is explained in
[Motion and animation architecture](../explanation/motion-and-animation-architecture.md).

## Quick Reference

```text
MJCF           = MuJoCo's native model format
body           = a rigid part in the body tree
freejoint      = lets the complete robot move freely
hinge          = rotates around one axis
inertial       = mass, centre of mass, and rotational inertia
visual geom    = shape drawn in the viewer
collision geom = shape used for contact calculations
site           = a massless measurement point on a body
actuator       = applies force or torque to control a joint
position error = difference between target and measured angle
kp             = strength pulling the joint toward the target
kv             = actuator resistance related to speed
forcerange     = maximum actuator torque
damping        = speed-related joint resistance
frictionloss   = dry joint friction
armature       = motor and gearbox inertia seen at the joint
backlash       = gear looseness, defined but not enabled here
```

The central idea is simple: Orion provides a target angle, but MuJoCo decides
the actual movement by simulating actuator strength, joint properties, mass,
gravity, and contact.
