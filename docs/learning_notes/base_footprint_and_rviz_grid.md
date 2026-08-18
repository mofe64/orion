# Orion's Base Footprint and the RViz Grid

## The Problem We Saw

When we first opened Orion in RViz, the base looked as though it was underneath the floor.

Note -> The surface in RViz was not a real floor. It was only a grid drawn on an XY coordinate plane. RViz was using `base_link` as its fixed frame, so it drew the grid at `base_link`'s Z position of zero.
This is RViz's default grid behaviour, it draws the grid on the xy plane at z = 0 of its selected reference frame.
Our saved config for rviz used base_link as our fixed frame and used that fixed frame as the grid reference frame.

We measured the visual meshes inside `base_link` and found:

```text
Top of the base:     z =  0.0000 m
Bottom of the base:  z = -0.0418 m
RViz grid:           z =  0.0000 m
```

This meant the `base_link` frame was at the top of the physical base. The complete base extended 0.0418 m downward from that frame, so the grid passed across the top of the base.
So this means that the body of our base link extended from the base link coordinate frame downward along the z axis by -0.0418 m

## A Link Frame Is Not Its Mesh

Every URDF link has a coordinate frame. The frame is an invisible reference point with X, Y, and Z axes.

The visual and collision meshes are positioned relative to that frame. The frame does not have to be at the centre or bottom of the meshes. It can be placed at a useful mechanical position, such as a joint axis.

In the original LeLamp model, the `base_link` frame was placed near the upper joint axis of the base. the joint connecting the base to the upper arm. This was valid for describing the mechanism, but it did not represent the surface supporting the lamp.


## The RViz Grid Is Not a Physical Floor

RViz displays information. It does not calculate gravity, collision forces, or contact with a floor.

The grid is only a visual reference. RViz draws it at the origin of its selected fixed frame. When the fixed frame was `base_link`, the grid was drawn through `base_link` at Z equals zero.

Moving the RViz grid alone could make the picture look correct, but it would not give the robot a coordinate frame that represents the real contact surface.

## The Fix

We added an empty link called `base_footprint`:

```xml
<link name="base_footprint"/>
```

`base_footprint` represents the point directly below `base_link` on the surface supporting Orion. Its Z position is the bottom of the base.

It has no visual, collision, or inertial section because it is not a physical robot part. It is only a coordinate frame. Orion's actual base mass and geometry remain in `base_link`.


We connected `base_footprint` to `base_link` using a fixed joint:

```xml
<joint name="base_footprint_joint" type="fixed">
  <origin xyz="0 0 0.0418" rpy="0 0 0"/>
  <parent link="base_footprint"/>
  <child link="base_link"/>
</joint>
```

The parent is `base_footprint`, and the child is `base_link`. The positive Z value places `base_link` 0.0418 m above `base_footprint`.

The base mesh still extends 0.0418 m downward from `base_link`. Therefore, its bottom now reaches Z equals zero in the `base_footprint` frame:

```text
base_link height above base_footprint =  0.0418 m
base bottom relative to base_link     = -0.0418 m
-------------------------------------------------
base bottom relative to footprint     =  0.0000 m
```

Because the joint is fixed, it cannot move. It adds no new controllable axis and does not add another slider to the joint-state GUI.

## How the TF Tree Changed

Before the fix, `base_link` was the root of Orion's TF tree. A root is the one link that has no parent.

After the fix, the beginning of the tree is:

```text
base_footprint
    -> base_footprint_joint
        -> base_link
```

`base_footprint` is now the root. All existing links remain below `base_link`, so the internal robot structure and all five movable joints remain unchanged.

`robot_state_publisher` publishes the fixed transform between `base_footprint` and `base_link` on `/tf_static`. The changing transforms created by the five movable joints continue to be published on `/tf`.

## What We Changed in RViz

We changed RViz's fixed frame and camera target from `base_link` to `base_footprint`.

RViz now draws its grid at the `base_footprint` origin. Since that origin is at the bottom of the base, Orion appears to rest on the grid.

This did not move individual meshes or change any joint. It changed the reference frame from which RViz views the complete robot.

## How This Affects Orion

The fix gives Orion a clear ground-contact reference frame. This makes it easier to understand the robot's height and place it on a floor or table later.

It also makes the root of the URDF massless. This avoids the KDL warning caused by placing inertial properties on the root link. The real mass is still stored correctly in `base_link`.

The fix does not physically attach Orion to a simulated floor. When we add Gazebo, we will still need to decide whether the lamp is fixed to the world or allowed to move under gravity. A coordinate frame describes a location; it does not create a physical constraint by itself.

The important distinction is:

- `base_footprint`: the ground-level reference frame
- `base_link`: the physical base assembly with meshes, collision geometry, mass, and inertia
- RViz grid: a visual plane drawn at the selected fixed frame
- Gazebo floor: future physical collision geometry used by the simulator
