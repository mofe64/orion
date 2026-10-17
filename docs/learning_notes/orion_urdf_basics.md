# URDF basics

The Unified Robot Description Format describes links, joints, shapes and limits. Orion's V1 model is [orion.urdf](../../description/urdf/orion.urdf). V2 uses a separate model.

## Links and frames

A link represents one rigid assembly. Its parts move together. A frame defines a position and orientation used as a reference.

Each link frame locates its meshes, center of mass and child joints. The frame does not need to coincide with the mesh center. A link's joint determines its position relative to its parent.

The root link has no parent. Later links form the robot's joint chain.

## Visual and collision shapes

A visual element supplies rendered geometry. A collision element supplies contact geometry. Their origins position shapes relative to the link frame.

Mesh paths are relative to the URDF file. The V1 meshes live in `description/meshes/`. Renaming a mesh requires changing its references.

## Origins and axes

`xyz` gives translation in metres. `rpy` gives roll, pitch and yaw in radians. Roll rotates around X, pitch around Y and yaw around Z.

A joint origin locates its child frame at zero joint displacement. Its axis gives the permitted motion direction in the joint frame. The joint angle then adds motion about that axis.

This fixed joint connects the V1 footprint and base:

```xml
<joint name="base_footprint_joint" type="fixed">
  <origin xyz="0 0 0.0418" rpy="0 0 0"/>
  <parent link="base_footprint"/>
  <child link="base_link"/>
</joint>
```

The translation raises the base frame by 0.0418 metres. A fixed joint has no commanded motion.

## Joint types and limits

A revolute joint rotates within lower and upper angle limits. A continuous joint rotates without URDF angle stops. A fixed joint connects parts without relative movement.

URDF limits describe the model. Runtime hardware configuration defines permitted physical commands. V1 model travel must not replace its calibrated command bounds.

## Mass and inertia

The inertial element contains mass, center of mass and inertia. Inertia describes resistance to rotational acceleration. Its six independent values form a symmetric matrix.

Mass uses kilograms. The inertial origin locates the center of mass relative to the link frame. It does not move the visible mesh.

## Combined motion

Every child transform depends on its parent. Thus, head direction depends on the full chain, not only the last joint. Forward kinematics computes these combined transforms from joint angles.

V2 uses mechanical-rest angles. Its URDF origins and MuJoCo references define the corresponding zero geometry. See [robot descriptions](../../description/README.md) and [joint structure](orion_joints.md).
