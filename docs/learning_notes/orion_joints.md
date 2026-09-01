# Orion's joint structure

Orion has five powered joints. Together they form one movement chain from the
base to the lamp head:

```text
base_link
    -> base_yaw_joint
shoulder_mount_link
    -> shoulder_pitch_joint
upper_arm_link
    -> elbow_pitch_joint
forearm_link
    -> head_roll_joint
head_roll_link
    -> head_pitch_joint
lamp_head_link
```

For every joint:

- the parent is the part before the joint;
- the child is the part after the joint;
- moving the joint moves its child and everything below that child;
- the parent does not move because of that joint.

The angles below are measured from the CAD-defined zero position. Zero is a
reference configuration, not automatically Orion's home or rest pose.

The approximate travel descriptions below explain the imported URDF geometry;
they are not permission to command that entire range on the physical robot.
The runtime's commissioned position bounds live in
[`motion/config/motion_limits.yaml`](../../motion/config/motion_limits.yaml),
and the hardware driver derives its conversion from the accepted servo
calibration.

## `base_yaw_joint`

```text
parent: base_link
child:  shoulder_mount_link
URDF reference travel: about 360 degrees
```

`base_link` contains the physical lamp base, base cover, and base-servo parts.
This joint turns around the base's vertical axis.

When `base_yaw_joint` moves, it turns every part above the base:

- shoulder mount;
- both arm sections;
- head support;
- complete lamp head.

The physical base itself remains facing the same direction.

## `shoulder_pitch_joint`

```text
parent: shoulder_mount_link
child:  upper_arm_link
URDF reference travel: about 180 degrees
```

`shoulder_mount_link` is the small bracket and servo assembly immediately above
the base. `upper_arm_link` is the first long arm section.

The joint axis is horizontal relative to the base. The joint raises and lowers
the first arm section. Because the rest of the chain is below `upper_arm_link`,
it also moves the elbow, forearm, and lamp head.

## `elbow_pitch_joint`

```text
parent: upper_arm_link
child:  forearm_link
URDF reference travel: about 180 degrees
```

This joint is the hinge between Orion's two long arm sections. It bends and
straightens the arm.

Moving it changes the position of the forearm and complete lamp head, but it
does not move the base or upper arm relative to the shoulder.

## `head_roll_joint`

```text
parent: forearm_link
child:  head_roll_link
URDF reference travel: about 360 degrees
```

This joint is at the end of the forearm, before the final lamp-head hinge. Its
axis runs approximately along the end of the arm in the exported pose.

It rolls the head-support assembly. Because `head_pitch_joint` and
`lamp_head_link` are below it, they roll with the support.

## `head_pitch_joint`

```text
parent: head_roll_link
child:  lamp_head_link
URDF reference travel: about 180 degrees
```

This is the final hinge in the chain. It tilts the complete lamp head relative
to its supporting bracket.

`lamp_head_link` contains both the diffuser and lamp-head geometry, so both
visible parts move together when this joint turns.

## How Combined Motion Works

Each joint angle is measured relative to its parent, but the final world pose
of the lamp head depends on every joint before it.

For example, a head tilt can look different after base yaw and elbow pitch have
already changed. The same `head_pitch_joint` angle is still used, but its parent
frame now has a different world position and orientation.

This is why Orion sends and records all five joint values together. Joint
names and parent-child relationships must agree across the URDF, MJCF, runtime,
and motion system. Their limits serve different purposes: the URDF explains the
imported model, while commissioned calibration and motion configuration
constrain physical execution.

The XML rules behind parent, child, origin, and axis are explained in
[URDF Basics](orion_urdf_basics.md).
