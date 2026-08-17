Links - a link represents a rigid assembly, everything isndiee it must move together without bending or rotating relative to other peices in that links.
a link can contain several CAD paarts, eg our lamparm__base_elbow contains 
 - the base to elbow arm
 - driving and passive sevro dics
 - sevro motot components
 - pcb/socker components
each part has its own visual and collisiton placment, but they all belong to the same link and therefore move as one rigid object. If one of those parts needed to move independenlty it would requires its own link connected by a joint.
A normal URDF Link does not specify its own overall position, its position comes from the joing connecting it to its parent. The one link without a parent becomes the root

Link Coordinate frame - every link has an invisinle cooridnate frame. THis link is the reference from which that links 
 - mesh positions
 - mesh rotations
 - center of mass
 - child joints
are measured. The link frame is not neccesarily at the centre of the mesh. it is often placed at a mechanically useful location

Visual Geometry - defines the rendered apperance of a link. The geometry tag specifies its visible shape while
origin positions and rotates that shape relative to the link's coordinate frame. It can also contain an optional material to control its color and texture.
A link may contain multiple visual elements when it cosists of several rigid CAD parts.

```XML
<visual>
  <origin
    xyz="0.0683802 0.000449515 0.192601"
    rpy="-5.09262e-15 -0.486539 2.83295"/>

  <geometry>
    <mesh filename="package://assets/lamparm__base_elbow.stl"/>
  </geometry>
</visual>
```
The above snippet contains the visual geom for one specifc cad part in a link
it essentially defines the following
 1. start at the links coordinate frame
 2. move the mesh by the specified xyz rotataion
 3. rotate it using the rpy
 4. draw the specified STL mesh there

URDF distances are normally in meters, so our xyz is approx 68.4 mm, 0.45 mm and 192.6 mm
It is important to note that `xyz` values do not position the entire link in the robot, They position this particular mesh relative to the links frame

`rpy` mesns roll -> rortation around x, pitch -> rotation around Y and Yaw -> rotation around z
Its values are radians, not degrees

Collision Geometry - defines the solid shape that a physics engine uses to detect contact between a link and other objects.
Its geometry specifies the collision shape, while origin positions and rotates that shape relative to the link frame

```XML
<link name="arm_link">
  <collision>
    <origin xyz="0 0 0.1" rpy="0 0 0"/>

    <geometry>
      <mesh filename="package://orion_description/meshes/arm.stl"/>
    </geometry>
  </collision>
</link>
```

A link cna have  multiple collistion elements, one for each solid component in the link
All collision shapres inside the link remain rigidly attached and move together.
It is important to note that collision geometry does not define the link's mass, that is handled separately by inertial


Inertial Properties - descrive how a link responds to forces and rotationsl motion in a phsics simulation.
The mass property soecifies its total mass, the origin locates the center of mass, and the inertia tensor descrives how its mass is distributed around that center

```XML
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

A link normally has one inertial section containing three parts
Inertial Origin - which places the link's center of mass relaive to its link frame using the xyz values
The RPY values give the orientaion of the inertial frame
Note -> The center of mass if the effective balance point of the complete rigid link not the center or origin of one mesh

Our mass value is specifiedin kg and represents the combined mass of everything grouped into the link

Inertia tensor - these values form a symmetric matrix
```Plain Text
┌                 ┐
│ ixx   ixy   ixz │
│ ixy   iyy   iyz │
│ ixz   iyz   izz │
└                 ┘
```

Their units are kg·m².
 - ixx describes resistance to rotation around X
 - iyy describes resistance to rotation around Y
 - izz describes resistance to rotation around Z
 - ixy, ixz, and iyz describe coupling between axes caused by an asymmetric or diffently oriented mass distribution

An object can have the same mass but behave differently depending on its interia. For example, a long arm is easier to rotate around its length than 
around an axis through one end  perpendicular to its length

The key distiction for links is 
visual    = what the link looks like
collision = where the link can make contact
inertial  = how the link responds to forces
These three descriptions belong to the same rigid link but serve different purposes.




It is important to note the followwing :
Link frame and intertial frames are conceptually separate coordinate frames.
Link frame: the main reference frame for the entire link.
Inertial frame: the frame used for the link’s centre of mass and inertia tensor.
The inertial frame is defined relative to the link frame:
<inertial>
  <origin xyz="0.0729066 -0.0229022 0.217607"
          rpy="0 0 0"/>
</inertial>


For Orion we have seven links
 - lamparm__base_elbow
 - lamparm__wrist_head
 - scs215_v5
 - imu_site
 - lamparm__elbow_wrist
 - lamparm__wrist_head_2
 - diffuser


 Five are major mechanical assemblies, diffuser also contains the lamp-head geometry, and imu_site is a tiny dummy link used to provide an IMU coordinate frame.


Joints
A joint defines
 - which link is the parent
 - which link is the child
 - where the joint is located
 - how it is oriented
 - what movement is allowed
 - how far and how quickly it can move
For example

```XML

<joint name="2" type="revolute">
  <origin xyz="0.00583234 0.0182927 0.08291"
          rpy="-1.26215 1.5708 0"/>

  <parent link="lamparm__base_elbow"/>
  <child link="lamparm__wrist_head"/>

  <axis xyz="0 0 1"/>

  <limit effort="10"
         velocity="10"
         lower="-1.08426"
         upper="2.05734"/>
</joint>
```

We can essentially think og a joint as a door hinge attached to the parent link

The parent and child relation is set as 
```XML
<parent link="lamparm__base_elbow"/>
<child link="lamparm__wrist_head"/>
```

THis means that lamparm_base_elbow -> joint 2 -> lamparm_wrist_head
when joint 2 moves, its child lamparm_wrist_head moves relative to the parent, everything below the child also moves
The propagation rule is fundamental -> MOving a joint moves it child link and every descendant of that child, it does not move the parent or the parent's other branches

Before the door hinge(joint) can rotate, URDF must answer two questions
1. where is the hinge attached ?
2. which direction for the hinge point ?

This is what the joint origin tells us

The joint origin - describes the joint's zero position frane relative to the parent link.
XYZ -> tells us where is the hinge

For our joint 2 in the above example, the joint is approximately
x = 5.8mm
y = 18.3mm
z = 82.9mm
from the lamparm__base_elbow frame
Those xyz values takes to the joint's pivot point
and these measurements are relative to the parent and not the world or the whole robot

RPY -> tells us how the hinge is oreiented
at the joint location, urdf rotates the joint's coordinate frame

```Plain Text
roll  = -72.3° around X
pitch =  90°   around Y
yaw   =   0°   around Z
```
This values point the joint's coordinate arrows in the correct physical direction
THis is neccessary because the hinge might be mounted sideways or diagonally. its rotation axis might not line up with the parent link's original X Y OR Z axes

Joint Axis - `<axis xyz="0 0 1"/>` 
This means rotate around the z axis of the newly positioned and rotated joint feature. 
This essentailly is the only allowed direction or axis the joint is allowed to move

Joint Angle Zero - this essentailly means when the command joint angle is zero, at this point the xyz translation and the rpy rotation is still applied.


Revolute Joint and limits
A revolute joint rotates around one axis and has lower and upper limits.
a continous joing would also rotate around one axis but would hae no angular limit.
Our Orion project uses bounded revolute joing for all five servos

Joint 2 allows:
lower = -1.08426 rad ≈ -62.1°
upper =  2.05734 rad ≈ 117.9°
Therefore, it has roughly 180° of total travel, but its range is offset around the CAD-defined zero position.

The other limit attributes we use are 
effort="10"
velocity="10"

for a revolute joint,
 - effort is nominaly maximum torque in N.m
 - velocity is maximum angular speed is rad/s

It is important that these values match the motor/sevro specifications


Fixed joints
Fixed joints allow no movements, in our orion urdf, we use a fixed joint for our IMU
`<joint name="imu_site_frame" type="fixed">` this permanently attached the imu_site to our sevros scs215_v5
