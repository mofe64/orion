# Orion's MuJoCo model

MuJoCo uses MJCF (its XML model format) to define bodies, joints, actuators, cameras and scenes. Orion's V2 model lives in [simulation/mujoco/v2/robot.xml](../../simulation/mujoco/v2/robot.xml) and is loaded by [scene.xml](../../simulation/mujoco/v2/scene.xml), while V1 lives in [simulation/mujoco/robot.xml](../../simulation/mujoco/robot.xml).

## Robot and scene

`robot.xml` defines the lamp mechanism itself, while `scene.xml` wraps it with `<include file="robot.xml" />` so different scenes can reuse the same robot definition.

## Compiler settings

The V2 model sets radians and explicit inertia properties:

```xml
<compiler angle="radian" inertiafromgeom="false" />
```

Because `inertiafromgeom="false"` is set, MuJoCo uses the CAD-exported `<inertial>` tags on each body rather than estimating mass from mesh volumes.

## Bodies, joints and coordinate zero

Nested `<body>` elements form the kinematic chain from `v21_base` up to `v21_neck_swivel_link`. Each `<joint type="hinge">` adds one rotational degree of freedom around its `axis`, and its `ref` attribute aligns the joint's `0`-radian angle with Orion's folded mechanical rest pose—if you change Orion's rest zero, the `ref` and URDF origins must stay synchronized.

On V2, `v21_base` is fixed in place (`orion_simulation_scope="kinematic_preview"`), so MuJoCo follows commanded trajectories kinematically without simulating base tipping or full lamp dynamics. V1 instead uses a `<freejoint>` so the simulated lamp stands freely on the floor under gravity.

## Shapes, sites and cameras

Visual `<geom>` meshes render the lamp parts (with collision checking disabled in V2's `<default>` block: `contype="0" conaffinity="0"`, while the V2 movement preview script inspects non-adjacent mesh contacts separately). Named `<site>` markers (`v21_face` and `light_frame`) and `<camera name="camera_optical_frame" ... />` inside the head let preview and vision tools query head height and gaze direction directly.

## Actuators and the runtime boundary

Each of the five joints has a `<position>` actuator (`kp="50" kv="2"`) that drives the joint toward the angle requested by `oriond`.

MuJoCo implements the exact same Rust driver interface as the physical STS3215 servos: both receive identical 50 Hz trajectory samples and report measured positions back for settling checks. Simulator settings never replace physical servo limits or mechanical zeroing. See [simulator setup](../../simulation/mujoco/README.md) and [URDF concepts](orion_urdf_basics.md).
