# URDF basics

URDF (Unified Robot Description Format) is an XML file that describes a robot's rigid parts, how they connect, where sensors sit, and how far joints can move. Orion's V2 model lives in [orion-v2.urdf](../../description/urdf/orion-v2.urdf) (with V1 kept separately in [orion.urdf](../../description/urdf/orion.urdf)).

## Links and frames

A **link** represents one rigid assembly whose parts move together (such as the base or the lower arm). Each link carries its own coordinate **frame**, locating its 3D meshes, centre of mass and any joints attached to it. A link's frame does not have to sit in the middle of its mesh—a joint's origin places the link relative to its parent.

## Visual shapes and meshes

Each `<visual>` block points to a mesh file (`STL` or `OBJ`) used to render the part. Mesh paths in `orion-v2.urdf` are relative to the URDF file (pointing into `../../simulation/mujoco/v2/meshes/`), so moving or renaming a mesh requires updating the URDF.

## Origins and axes

URDF positions use `xyz` in metres and rotations use `rpy` (roll around X, pitch around Y, yaw around Z) in radians.

A joint's `<origin>` places the child frame when the joint angle is `0` radians—which on V2 corresponds directly to the folded mechanical-rest pose—and `<axis>` specifies which axis the joint rotates around. Here is the base yaw joint from `orion-v2.urdf`:

```xml
<joint name="base_yaw_joint" type="revolute">
  <parent link="v21_base" />
  <child link="v21_base_yaw_link" />
  <origin xyz="0.0 0.0 0.060000000000000005" rpy="0.0 0.0 -0.04908738521234052" />
  <axis xyz="0 0 -1" />
  <limit lower="-1.4879613642490719" upper="1.4910293258248433" effort="0" velocity="0" />
</joint>
```

## Joint types and limits

- **`revolute`:** Rotates around its axis between `lower` and `upper` angle limits (used for all five powered joints).
- **`fixed`:** Locks two frames rigidly together with no movement (used on V2 to place `camera_optical_frame` and `light_frame` inside the head):

```xml
<joint name="camera_optical_joint" type="fixed">
  <parent link="v21_neck_swivel_link" />
  <child link="camera_optical_frame" />
  <origin xyz="0 -0.082 0.0157" rpy="-1.5707963267948966 0 3.141592653589793" />
</joint>
```

While URDF `<limit>` tags mirror the hardware profile, `hardware/profiles/v2-joints.json` remains the authoritative source for runtime command limits (just as calibration defines bounds on V1).

## Mass and inertia

Each `<inertial>` block lists a link's mass (kg), centre of mass and symmetric `3×3` inertia matrix (resistance to rotational acceleration). These properties are used by physics tools without affecting the visual meshes.

## Combined motion

Because every child transform builds on its parent, forward kinematics multiplies the chain of transforms from `v21_base` up to `v21_neck_swivel_link` to compute where the camera and light actually point in space. See [robot descriptions](../../description/README.md) and [joint structure](orion_joints.md).
