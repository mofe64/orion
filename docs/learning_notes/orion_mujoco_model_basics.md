# How Orion's MuJoCo Model Works

MuJoCo needs a model that describes Orion's bodies, joints, mass, shapes,
motors, and sensors. That model uses MuJoCo's XML format, usually called MJCF.

## The Two XML Files

Orion keeps the robot and the surrounding scene separate:

```text
simulation/mujoco/robot.xml  robot bodies, joints, motors, and sensors
simulation/mujoco/scene.xml  floor, light, camera settings, and physics step
```

`scene.xml` includes `robot.xml`. This lets us place the same robot model in a
different scene without copying it.

## Units and Meshes

At the top of `robot.xml`:

```xml
<compiler angle="radian" meshdir="assets" autolimits="true"/>
```

- `angle="radian"` means joint angles use radians.
- `meshdir="assets"` tells MuJoCo where the STL files live.
- `autolimits="true"` lets MuJoCo use a joint's range as its limit.

These units must agree with Orion's ROS motion files.

## Bodies Form a Tree

A MuJoCo `<body>` is similar to a URDF link. Bodies are nested to form a tree.
When a joint moves, its child body and everything below that child move.

Each body can contain:

- an inertial section for mass and rotational inertia;
- visual geometry that we see;
- collision geometry used by physics;
- a joint that allows movement relative to its parent.

`pos` gives a location relative to the parent body. `quat` gives orientation as
a quaternion in `w x y z` order.

## Free Root and Ground Contact

Orion's root body has a `freejoint`. A free joint allows the whole model to
translate and rotate in space. Gravity and contact can therefore affect the
base.

The base uses a simple flat box for reliable floor contact:

```xml
<geom name="base_floor_collision" type="box" .../>
```

The detailed meshes still provide the visible shape. A simple collision shape
is usually more stable and cheaper for physics than a complex triangle mesh.

The imported body tree does not begin at the physical base. When the native
player sets an initial joint pose, changing the joints could move the base as a
side effect. `mujoco_backend.py` corrects the free-root transform so the
physical base body stays at the same world pose.

## Orion's Five Hinges

Orion has five controlled hinge joints:

```text
base_yaw_joint
shoulder_pitch_joint
elbow_pitch_joint
head_roll_joint
head_pitch_joint
```

A hinge rotates around one axis and stays inside its configured range. MuJoCo,
ROS, and the motion package use these same semantic names.

Code looks up joints and actuators by name. It does not trust XML array order.
This prevents a reordered model from sending a command to the wrong joint.

## Mass and Inertia

Physics needs more than visible shapes. Each moving body also has:

- `mass`: how heavy it is;
- inertial `pos`: where its centre of mass is;
- `fullinertia`: how hard it is to rotate around different axes.

Bad values can make a model fall, shake, or accelerate unrealistically even
when it looks correct.

## Visual and Collision Geometry

Visual geometry controls appearance. Collision geometry controls contact.
They can use the same mesh, but they do not have to.

Orion uses MJCF default classes to avoid repeating settings:

```xml
<default class="visual">
  <geom type="mesh" contype="0" conaffinity="0"/>
</default>
```

The visual class does not take part in collision. Collision-class geometry is
used by physics.

## Position Actuators

Each moving joint has a position actuator with the same name:

```xml
<position class="sts3215"
          name="base_yaw_joint"
          joint="base_yaw_joint"
          inheritrange="1"/>
```

The control value is a target angle. The actuator pushes the joint toward that
angle.

Important actuator settings are:

- `kp`: how strongly position error creates force;
- `kv`: speed-based resistance inside the actuator;
- `forcerange`: the maximum positive and negative actuator force;
- `inheritrange="1"`: use the joint range as the allowed control range.

Orion's `sts3215` class uses `kp="17.8"` and limits force to
`-3.35 ... 3.35`. These are simulator settings. They do not prove that the
model exactly matches a physical STS3215 servo.

## Passive Joint Behaviour

Joint settings also affect movement without changing the requested target:

- `damping` resists speed;
- `frictionloss` resists movement;
- `armature` adds reflected motor inertia.

These values strongly affect overshoot, settling, and stability. They should
be changed with a test and a reason, not only because one animation looks
better.

## Sensors and Sites

A site is a named position and orientation attached to a body. Orion has an
`imu_site` on its base.

The model attaches two sensors to that site:

- an accelerometer;
- a gyroscope.

As the base moves, the site moves with it, so the readings describe the IMU's
location on Orion.

## Native MuJoCo and ROS-Controlled MuJoCo

The model can be driven in two ways:

- The native player writes position targets directly to MuJoCo actuators.
- `mujoco_ros2_control` will connect the same joints to ROS controllers.

The native route is fast and useful for model tests. The ROS-controlled route
will make MuJoCo accept the same `FollowJointTrajectory` interface as Gazebo.
Both routes use the same robot physics model and semantic joint names.

The shared movement path is explained in
[How Orion Moves](orion_motion_system.md).
