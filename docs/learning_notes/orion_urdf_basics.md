# URDF basics

[`description/urdf/orion.urdf`](../../description/urdf/orion.urdf) defines
Orion's link geometry, frames, axes, and model reference ranges. Physical servo
calibration and runtime safety limits are separate.

## Links

A link represents a rigid assembly. Everything inside it must move together without bending or rotating relative to the other pieces in that link.

A link can contain several CAD parts. For example, our `upper_arm_link` contains:

- The base-to-elbow arm
- Driving and passive servo discs
- Servo motor components
- PCB/socket components

Each part has its own visual and collision placement, but they all belong to the same link and therefore move as one rigid object. If one of those parts needed to move independently, it would require its own link connected by a joint.

A normal URDF link does not specify its own overall position. Its position comes from the joint connecting it to its parent. The one link without a parent becomes the root.

## Link Coordinate Frame

Every link has an invisible coordinate frame. This frame is the reference from which that link's following properties are measured:

- Mesh positions
- Mesh rotations
- Centre of mass
- Child joints

The link frame is not necessarily at the centre of the mesh. It is often placed at a mechanically useful location.

## Visual Geometry

Visual geometry defines the rendered appearance of a link. The `<geometry>` tag specifies its visible shape, while `<origin>` positions and rotates that shape relative to the link's coordinate frame. It can also contain an optional material to control its colour and texture.

A link may contain multiple visual elements when it consists of several rigid CAD parts.

```xml
<visual>
  <origin
    xyz="0.06254786 -0.017843185 0.109691"
    rpy="-5.09262e-15 -0.486539 2.83295"/>

  <geometry>
    <mesh filename="../meshes/lamparm__base_elbow.stl"/>
  </geometry>
</visual>
```

The snippet above contains the visual geometry for one specific CAD part in a link. It essentially defines the following:

1. Start at the link's coordinate frame.
2. Move the mesh by the specified XYZ translation.
3. Rotate it using the RPY values.
4. Draw the specified STL mesh there.

URDF distances are normally in metres, so our XYZ values are approximately 62.5 mm, -17.8 mm, and 109.7 mm.

It is important to note that `xyz` values do not position the entire link in the robot. They position this particular mesh relative to the link's frame.

`rpy` means:

- Roll: rotation around X
- Pitch: rotation around Y
- Yaw: rotation around Z

Its values are radians, not degrees.

## Collision Geometry

Collision geometry defines the solid shape that a physics engine uses to detect contact between a link and other objects. Its geometry specifies the collision shape, while `<origin>` positions and rotates that shape relative to the link frame.

```xml
<link name="arm_link">
  <collision>
    <origin xyz="0 0 0.1" rpy="0 0 0"/>

    <geometry>
      <mesh filename="../meshes/arm.stl"/>
    </geometry>
  </collision>
</link>
```

A link can have multiple collision elements, one for each solid component in the link. All collision shapes inside the link remain rigidly attached and move together.

It is important to note that collision geometry does not define the link's mass. That is handled separately by `<inertial>`.

## Inertial Properties

Inertial properties describe how a link responds to forces and rotational motion in a physics simulation. The mass property specifies its total mass, the origin locates the centre of mass, and the inertia tensor describes how its mass is distributed around that centre.

```xml
<inertial>
  <origin xyz="0.06707426 -0.0411949 0.134697"
          rpy="0 0 0"/>

  <mass value="0.234138"/>

  <inertia
    ixx="0.0010919"
    ixy="7.35843e-05"
    ixz="-0.000504692"
    iyy="0.00129476"
    iyz="0.000161616"
    izz="0.000320305"/>
</inertial>
```

A link normally has one inertial section containing three parts.

### Inertial Origin

The inertial origin places the link's centre of mass relative to its link frame using the XYZ values. The RPY values give the orientation of the inertial frame.

The centre of mass is the effective balance point of the complete rigid link, not the centre or origin of one mesh.

Our mass value is specified in kilograms and represents the combined mass of everything grouped into the link.

### Inertia Tensor

These values form a symmetric matrix:

```text
┌                 ┐
│ ixx   ixy   ixz │
│ ixy   iyy   iyz │
│ ixz   iyz   izz │
└                 ┘
```

Their units are kg·m².

- `ixx` describes resistance to rotation around X.
- `iyy` describes resistance to rotation around Y.
- `izz` describes resistance to rotation around Z.
- `ixy`, `ixz`, and `iyz` describe coupling between axes caused by an asymmetric or differently oriented mass distribution.

An object can have the same mass but behave differently depending on its inertia. For example, a long arm is easier to rotate around its length than around an axis through one end that is perpendicular to its length.

The key distinction for links is:

- Visual: what the link looks like
- Collision: where the link can make contact
- Inertial: how the link responds to forces

These three descriptions belong to the same rigid link but serve different purposes.

It is important to note the following:

- Link frame: the main reference frame for the entire link.
- Inertial frame: the frame used for the link's centre of mass and inertia tensor.

The link frame and inertial frame are conceptually separate coordinate frames. The inertial frame is defined relative to the link frame:

```xml
<inertial>
  <origin xyz="0.06707426 -0.0411949 0.134697"
          rpy="0 0 0"/>
</inertial>
```

## Orion's Links

Orion has eight links:

- `base_footprint`
- `upper_arm_link`
- `shoulder_mount_link`
- `base_link`
- `imu_link`
- `forearm_link`
- `head_roll_link`
- `lamp_head_link`

Five are major mechanical assemblies. `lamp_head_link` also contains the diffuser geometry. `base_footprint` and `imu_link` are empty links used to provide useful coordinate frames.

## Joints

A joint defines:

- Which link is the parent
- Which link is the child
- Where the joint is located
- How it is oriented
- What movement is allowed
- How far and how quickly it can move

For example:

```xml
<joint name="shoulder_pitch_joint" type="revolute">
  <origin xyz="-0.04111 0 0.0192"
          rpy="1.57079749794 -0.308646326793 -1.57080018218"/>

  <parent link="shoulder_mount_link"/>
  <child link="upper_arm_link"/>

  <axis xyz="-0.3037692089 -0.952745646919 1.11580660844e-06"/>

  <limit effort="10"
         velocity="10"
         lower="-1.08426"
         upper="2.05734"/>
</joint>
```

We can essentially think of a joint as a door hinge attached to the parent link.

The parent-and-child relationship is set as:

```xml
<parent link="shoulder_mount_link"/>
<child link="upper_arm_link"/>
```

This means:

```text
shoulder_mount_link -> shoulder_pitch_joint -> upper_arm_link
```

When `shoulder_pitch_joint` moves, its child, `upper_arm_link`, moves relative to the parent. Everything below the child also moves.

The propagation rule is fundamental: moving a joint moves its child link and every descendant of that child. It does not move the parent or the parent's other branches.

### Orion's Moving Joints

| Joint | What it moves |
|---|---|
| `base_yaw_joint` | Turns the lamp around the base's vertical axis. |
| `shoulder_pitch_joint` | Raises and lowers the first arm section. |
| `elbow_pitch_joint` | Bends and straightens the arm. |
| `head_roll_joint` | Rolls the head support at the end of the arm. |
| `head_pitch_joint` | Tilts the complete lamp head. |

This table describes their jobs. The URDF remains the source for their exact
parent, child, axis, and movement limits.

Before the door hinge (joint) can rotate, URDF must answer two questions:

1. Where is the hinge attached?
2. In which direction does the hinge rotate?

This is what the joint origin tells us.

## Joint Origin

The joint origin describes the joint's zero-position frame relative to the parent link.

### XYZ: Where Is the Hinge?

For our `shoulder_pitch_joint` example, the joint is approximately:

```text
x = -41.1 mm
y =   0.0 mm
z =  19.2 mm
```

from the `shoulder_mount_link` frame. Those XYZ values take us to the joint's pivot point. These measurements are relative to the parent, not the world or the whole robot.

### RPY: How Is the Hinge Oriented?

At the joint location, URDF rotates the joint's coordinate frame:

```text
roll  =  90.0° around X
pitch = -17.7° around Y
yaw   = -90.0° around Z
```

These values point the joint's coordinate arrows in the correct physical direction. This is necessary because the hinge might be mounted sideways or diagonally. Its rotation axis might not line up with the parent link's original X, Y, or Z axes.

## Joint Axis

```xml
<axis xyz="0 0 1"/>
```

This means to rotate around the Z axis of the newly positioned and rotated joint frame. This is the only axis around which the joint is allowed to move.

## Joint Angle Zero

When the commanded joint angle is zero, the XYZ translation and RPY rotation are still applied.

## Revolute Joints and Limits

A revolute joint rotates around one axis and has lower and upper limits. A continuous joint also rotates around one axis but has no angular limit. Our Orion project uses bounded revolute joints for all five servos.

`shoulder_pitch_joint` allows:

```text
lower = -1.08426 rad ≈ -62.1°
upper =  2.05734 rad ≈ 117.9°
```

Therefore, it has roughly 180° of total travel, but its range is offset around the CAD-defined zero position.

The other limit attributes we use are:

```xml
effort="10"
velocity="10"
```

For a revolute joint:

- `effort` is nominally the maximum torque in N·m.
- `velocity` is the maximum angular speed in rad/s.

In a production URDF, these values should describe meaningful actuator limits.
Orion's imported `effort="10"` and `velocity="10"` values are generic
placeholders; they are not commissioned STS3215 limits and must not be used to
infer safe hardware commands. Physical position conversion comes from the
accepted servo calibration, while tracked operational and provisional dynamic
limits live in
the active Pi calibration (or its tracked MuJoCo copy for offline work).

## Fixed Joints

Fixed joints allow no movement. In our Orion URDF, we use fixed joints for our ground-contact and IMU frames:

```xml
<joint name="base_footprint_joint" type="fixed">
```

This places `base_link` above `base_footprint` while keeping them rigidly attached.

```xml
<joint name="imu_fixed_joint" type="fixed">
```

This permanently attaches `imu_link` to `base_link`.
