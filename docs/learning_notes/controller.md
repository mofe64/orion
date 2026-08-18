Our orion_controller.yaml outlines which controllers should exist in out system and how should they behave
`controller_manager` is responsible for 
    - laoding controller plugins
    - configuring controllers
    - activating and deactivating conrollers
    - running the controller update loop
    - managing access to hardware interfaces
    - preventing multiple controllers from commanding the same hardware interface at the same time
    - reading joint state from the hardware backend
    - passing controller commands to the hardware backend

Note -> for our gazebo sim, we don't explicitly create the controller mnger node in our launch file, the `gz_ros2_control` gazebo plugin creats it for us

The controller_manager section of the yaml must match our actual controller manager node name, else ros would not be able to apply the settigs and params, so if we rename our node to orion_controller_manager, we need to update our yaml to be orion_controller_manager

`ros_parameters` under `controller_manager` defines the params that we want to pass to our controller_manager node.
We provide the following params
`update_rate:100` -> runs our controller manager's controll loop at 100hz. during each update what we are essentialy doing is 
1. read joint state from gazebo
2. update active controllers
3. calculate new commands
4. write commands back to gazebo

Gazebo itself runs the stepping physics at 1000hz
`enforce_command_limits:true` -> configures controller to enforce the joint limits from orion's robot description
with enforcement enableed, a position command outside that range is clamped to the allowed range before being passed to our simulated hardware
It does this using satuaration limiters whih will keep the command at the nearest valid boundary and not stop the entire simulation

`joint_state_broadcaster` -> this declares a controller named joint state broadcaster with the type of `joint_state_broadcaster/JointStateBroadcaster` naming the controller the same name as the type makes it easier to read, but we can name it anything
the type value we supply tells the controller manager which compiled controller plugin to load. 
In our case, the Joint state broadcaster reads state interfaces from the hardware backnd, for orion these are 
    - base_yaw_joint/position
    - base_yaw_joint/velocity
    - base_yaw_joint/effort
    - ...

and it pubslied those values to the `/joint_states` topic
The values it pubslised are measurements coming from the robot, not just requestd targets, the broadcaster does not move anything. it just reports state.
we use the default behavior for the broadcaster, so no need for a config section for this controller


`joint_trajectory_controller` -> this decalres a controller names joint trajectory controller. This controller is responsible for motion. It accepts a trajectory containing
1. Joint names
2. desired joint positions
3. times at which those positions should be reached
4. optionally desired velocity and acceleration

it then generates intermediate commands during the motion

For example if our end destination is 
start : base yaw approx 0.0 rad
goal : base yaw 0.10 rad
time: 3 seconds

the controller will not immediately jump from 0.0 to 0.10, it will generate intermediate position commands over the requested duration


`joint_trajectory_controller` section -> THis is the cofigurration for our loaded joint trajectory controller, in here we specify the joints which the controller owns using `joints`. 

It is important to note the following must match exactly
1. the joint names in the urdf
2. the joint names in the `<ros2_control>` section og the urdf
3. the names used in trajectory commands

THe supplied joints list ot our joint trajectory controller defines the order used in the controller. when the controller becomes active, it will claim the position interface belonging to all joints we supplied in this list.
Claiming an interface just means that the controller has exclusive permission to write command to the interface

`command_interfaces` -> this tells our trajectory controller what kind of command it is allowed to send.  in this instace we selected position commands. other possible options may be velocity -> eg rotate at 0.2 radians per second or effor -> apply a particular torque
For orion we only expose joint_name/position as a command interface, we do this in the ros2 control section for every joint
```xml
<joint name="base_yaw_joint">
  <command_interface name="position">
    <param name="min">-5.02103</param>
    <param name="max">1.26215</param>
  </command_interface>

  <state_interface name="position"/>
  <state_interface name="velocity"/>
  <state_interface name="effort"/>
</joint>
```

Note -> positon command and position state are different
position command - where we want the joint to go
position state - where the joint currently is

while the joint is moving they might briefly differ


`state_interfaces` -> these are the measurements that our trajectory conroller needs to operate,  we read the current joint position and the current joint velocity
The controller will use these measurements to monitor movement and determine if a trajectory goal has been reached.

Our urdf aslo exposes effor state, but the trajectory controller does not requires effort for our position command config

`allow_partial_joints_goal:false` -> this measthat every trajectory goal must include all five configured joints

A command containing only this:
joint_names:
  - base_yaw_joint
would be rejected.

The command must include:
joint_names:
  - base_yaw_joint
  - shoulder_pitch_joint
  - elbow_pitch_joint
  - head_roll_joint
  - head_pitch_joint

but we can supply zero targts for each joint we don't want to explicitly move eg 
base yaw:       0.10
shoulder pitch: 0.00
elbow pitch:    0.00
head roll:      0.00
head pitch:     0.00


