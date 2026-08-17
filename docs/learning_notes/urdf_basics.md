# URDF Basics

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
    xyz="0.0683802 0.000449515 0.192601"
    rpy="-5.09262e-15 -0.486539 2.83295"/>

  <geometry>
    <mesh filename="package://orion_description/meshes/lamparm__base_elbow.stl"/>
  </geometry>
</visual>
```

The snippet above contains the visual geometry for one specific CAD part in a link. It essentially defines the following:

1. Start at the link's coordinate frame.
2. Move the mesh by the specified XYZ translation.
3. Rotate it using the RPY values.
4. Draw the specified STL mesh there.

URDF distances are normally in metres, so our XYZ values are approximately 68.4 mm, 0.45 mm, and 192.6 mm.

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
      <mesh filename="package://orion_description/meshes/arm.stl"/>
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
  <origin xyz="0.0729066 -0.0229022 0.217607"
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
  <origin xyz="0.0729066 -0.0229022 0.217607"
          rpy="0 0 0"/>
</inertial>
```

## Orion's Links

Orion has seven links:

- `upper_arm_link`
- `shoulder_mount_link`
- `base_link`
- `imu_link`
- `forearm_link`
- `head_roll_link`
- `lamp_head_link`

Five are major mechanical assemblies. `lamp_head_link` also contains the diffuser geometry, and `imu_link` is a tiny dummy link used to provide an IMU coordinate frame.

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
  <origin xyz="0.00583234 0.0182927 0.08291"
          rpy="-1.26215 1.5708 0"/>

  <parent link="upper_arm_link"/>
  <child link="shoulder_mount_link"/>

  <axis xyz="0 0 1"/>

  <limit effort="10"
         velocity="10"
         lower="-1.08426"
         upper="2.05734"/>
</joint>
```

We can essentially think of a joint as a door hinge attached to the parent link.

The parent-and-child relationship is set as:

```xml
<parent link="upper_arm_link"/>
<child link="shoulder_mount_link"/>
```

This means:

```text
upper_arm_link -> shoulder_pitch_joint -> shoulder_mount_link
```

When `shoulder_pitch_joint` moves, its child, `shoulder_mount_link`, moves relative to the parent. Everything below the child also moves.

The propagation rule is fundamental: moving a joint moves its child link and every descendant of that child. It does not move the parent or the parent's other branches.

Before the door hinge (joint) can rotate, URDF must answer two questions:

1. Where is the hinge attached?
2. In which direction does the hinge rotate?

This is what the joint origin tells us.

## Joint Origin

The joint origin describes the joint's zero-position frame relative to the parent link.

### XYZ: Where Is the Hinge?

For our `shoulder_pitch_joint` example, the joint is approximately:

```text
x = 5.8 mm
y = 18.3 mm
z = 82.9 mm
```

from the `upper_arm_link` frame. Those XYZ values take us to the joint's pivot point. These measurements are relative to the parent, not the world or the whole robot.

### RPY: How Is the Hinge Oriented?

At the joint location, URDF rotates the joint's coordinate frame:

```text
roll  = -72.3° around X
pitch =  90°   around Y
yaw   =   0°   around Z
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

It is important that these values match the motor/servo specifications.

## Fixed Joints

Fixed joints allow no movement. In our Orion URDF, we use a fixed joint for our IMU:

```xml
<joint name="imu_fixed_joint" type="fixed">
```

This permanently attaches `imu_link` to `base_link`.
