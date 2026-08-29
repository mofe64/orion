# Orion's Base Frame and the RViz Grid

Orion uses two different frames near its base:

- `base_footprint` is a ground-level reference frame.
- `base_link` is the physical base, with shape, mass, and inertia.

They are related, but they do different jobs.

## A Frame Is an Invisible Reference

A frame is a point with X, Y, and Z directions. Other parts can describe their
position relative to it.

A link's frame does not have to sit at the centre or bottom of its mesh. For
example, `base_link` uses a useful mechanical position near the top of the
base. Its mesh extends `0.0418 m` below that point.

```text
base_link frame:    z =  0.0000 m
bottom of its mesh: z = -0.0418 m
```

The frame is not wrong. It simply does not represent the surface under Orion.

## What `base_footprint` Means

`base_footprint` marks the bottom of Orion's base:

```xml
<link name="base_footprint"/>
```

It is an empty link. It has no mesh, collision shape, mass, or inertia because
it is a reference, not a physical part.

A fixed joint places `base_link` above it:

```xml
<joint name="base_footprint_joint" type="fixed">
  <origin xyz="0 0 0.0418" rpy="0 0 0"/>
  <parent link="base_footprint"/>
  <child link="base_link"/>
</joint>
```

The calculation is:

```text
base_link above base_footprint =  0.0418 m
mesh bottom below base_link    = -0.0418 m
-------------------------------------------
mesh bottom above footprint    =  0.0000 m
```

Because the joint is fixed, it adds no movement to the robot.

## The Start of the TF Tree

The transform tree begins like this:

```text
base_footprint
    -> base_footprint_joint
        -> base_link
```

`robot_state_publisher` publishes this fixed relationship on `/tf_static`.
Transforms for Orion's five moving joints are published on `/tf`.

## What the RViz Grid Means

The RViz grid is only a drawing. It is not a floor and cannot support the
robot.

RViz draws the grid at height zero in the selected reference frame. Orion's
RViz configuration uses `base_footprint`, so the grid appears at the bottom of
the base.

Changing the RViz reference frame changes the view. It does not move meshes or
change joint positions.

## A Frame Is Not a Physical Constraint

These ideas are easy to mix up:

- `base_footprint`: an invisible ground-level reference;
- `base_link`: Orion's physical base assembly;
- RViz grid: a visual guide;
- simulator floor: collision geometry used by physics;
- fixed-to-world constraint: a rule that prevents the base from moving.

Adding `base_footprint` does not bolt Orion to a table. A simulator must still
use contacts or a fixed constraint to decide how the base behaves physically.
