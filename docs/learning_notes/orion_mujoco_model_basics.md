# Orion's MuJoCo model

MuJoCo uses MJCF, its XML model format, for bodies, joints, actuators, sensors and contact geometry. Orion's V1 model lives in [robot.xml](../../simulation/mujoco/robot.xml).

## Robot and scene

`robot.xml` contains the mechanism. `scene.xml` contains the floor, lights, cameras and simulation settings. The scene includes the robot file:

```xml
<include file="robot.xml"/>
```

Separating these files permits different environments to use the same mechanism.

## Compiler settings

The V1 model uses these compiler settings:

```xml
<compiler angle="radian" meshdir="../../description/meshes" autolimits="true"/>
```

Angles use radians. Mesh names resolve within `description/meshes/`. `autolimits` applies limits when a range is present.

## Bodies and joints

A body defines a local frame, mass and attached shapes. Nested bodies form the movement chain. A hinge joint adds one rotational degree of freedom. A degree of freedom is one independent movement coordinate.

A free joint permits translation and rotation of the base. V1 uses a free-standing base with floor contact. V2 instead fixes its base and follows kinematic targets.

The joint `ref` value defines the reference coordinate. Geometry depends on joint position minus that reference. Changing coordinate zero therefore requires corresponding reference changes.

## Shapes and contact

Visual meshes supply appearance. Collision shapes supply contact. `contype` and `conaffinity` control which shapes can collide.

Detailed visual geometry does not need identical collision geometry. Simpler contact shapes can reduce computation. Contact settings and mass properties affect simulated behavior independently of the motion compiler.

## Actuators and sensors

A position actuator applies force toward a requested joint position. Its gain, damping and force range affect tracking. The runtime still supplies compiled targets.

Joint damping opposes velocity. Friction loss approximates resisting torque. Armature adds rotational inertia. These model values do not change physical servo registers.

Sensors provide model state such as acceleration and angular velocity. Sites define named locations for sensors or geometric queries.

## Runtime boundary

The MuJoCo driver implements the same control interface as the hardware driver. Both receive 50 Hz trajectory samples. Both report feedback for measured-settling logic.

Simulation settings do not replace hardware limits or mechanical setup. V2 kinematic tracking does not compute the complete lamp's physical dynamics.

See [simulator setup](../../simulation/mujoco/README.md) and [URDF concepts](orion_urdf_basics.md).
