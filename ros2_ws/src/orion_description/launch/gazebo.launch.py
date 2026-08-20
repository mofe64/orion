from pathlib import Path

from ament_index_python.packages import get_package_share_directory
from launch import LaunchDescription
# IncludeLaunchDescription is used to include another launch file in our launch file
from launch.actions import DeclareLaunchArgument, IncludeLaunchDescription
from launch.launch_description_sources import PythonLaunchDescriptionSource
from launch.substitutions import LaunchConfiguration
from launch_ros.actions import Node


BASE_ODOMETRY_TOPIC = '/orion/base_odometry'
BASE_ODOMETRY_BRIDGE = (
    f'{BASE_ODOMETRY_TOPIC}@nav_msgs/msg/Odometry[gz.msgs.Odometry'
)
BASE_CONTACT_TOPIC = '/orion/base_contacts'
GAZEBO_BASE_CONTACT_TOPIC = (
    '/world/empty/model/orion/link/base_footprint/'
    'sensor/base_contact_sensor/contact'
)
BASE_CONTACT_BRIDGE = (
    f'{GAZEBO_BASE_CONTACT_TOPIC}'
    '@ros_gz_interfaces/msg/Contacts[gz.msgs.Contacts'
)

# this launch file launches six launch actions
# Gazebo
# robot_state_publisher
# Orion spawning process
# clock bridge
# joint-state broadcaster spawner
# trajectory-controller spawner
# important to note that each action does not fully finish before the next one begines, they generally start concurrently
# the components coordianate by waiting for the resources they need.


def generate_launch_description():
    gz_args = LaunchConfiguration('gz_args')
    # locates the orion's insatlled pkg resources
    package_share = Path(get_package_share_directory('orion_description'))
    # locate the installed ros gazebo sim package
    ros_gz_sim_share = Path(get_package_share_directory('ros_gz_sim'))

    urdf_path = package_share / 'urdf' / 'orion.urdf'
    gazebo_launch_path = ros_gz_sim_share / 'launch' / 'gz_sim.launch.py'

    # read robpt desc
    # and replace the $(find orion_description) string with the abs path for our installed pkg in the gazebo section of our urdf, we do this because we are usng a basic urdf file
    # not xacro so we can't resolve find
    robot_description = urdf_path.read_text(encoding='utf-8').replace(
        '$(find orion_description)',
        str(package_share),
    )

    # create an action that brings in gazebo's launch file using the args
    # -r which starts the simulation running and allows the control loop to begin without it the controller activation will time out becasue controller activation needs the simulation update loop to run
    # empty.sdf which loads the gazebo empty world
    gazebo = IncludeLaunchDescription(
        PythonLaunchDescriptionSource(str(gazebo_launch_path)),
        launch_arguments={'gz_args': gz_args}.items(),
    )
    
    # create an action for our robot state publisder node
    # the name we use here is what will apper in our ros graph
    # we set output to screen to send the logs to our terminal withot it
    # useful start up logs go into ROS log files
    # we provide the robot desc param, robot state publishger users the urdf and the /joint_states to calculate the coordinate transforms between links 
    # and publsihed those transforms through /tf and /tf_static
    # use sim time tells the node to use gazebo's clocl instead of the computers wall clock this is important because
    # simulation may run slower than real time, run faster than real time, be pasued or reset
    # all sim nodes need to agree on what time is
    robot_state_publisher = Node(
        package='robot_state_publisher',
        executable='robot_state_publisher',
        name='robot_state_publisher',
        output='screen',
        parameters=[{
            'robot_description': robot_description,
            'use_sim_time': True,
        }],
    )

    # defines a short lived heler to create our orion robot in the gazebo world
    # the create executable just asks gazebo to create a model, it does nto manage the robot afterwards
    # name gives the gazebo model its name
    # topic tells it where to obtain the urdf
    # the 3 coords defint the inital world position
    spawn_orion = Node(
        package='ros_gz_sim',
        executable='create',
        name='spawn_orion',
        output='screen',
        parameters=[{
            'name': 'orion',
            'topic': '/robot_description',
            'x': 0.0,
            'y': 0.0,
            'z': 0.0,
        }],
    )

    # gazebo and ros use diff message systems, this node connects them
    # parameter_bridge exec cam translate selected topics between gazebo transport messages
    # and ros mesages
    # argument we provided heres is /topicname@rosMessageType[gazeboMessageType
    # '[' specifies a one way gazebo to ros bridge: Gazebo clock → ROS /clock
    simulator_bridge = Node(
        package='ros_gz_bridge',
        executable='parameter_bridge',
        name='simulator_bridge',
        output='screen',
        arguments=[
            '/clock@rosgraph_msgs/msg/Clock[gz.msgs.Clock',
            BASE_ODOMETRY_BRIDGE,
            BASE_CONTACT_BRIDGE,
        ],
        remappings=[(GAZEBO_BASE_CONTACT_TOPIC, BASE_CONTACT_TOPIC)],
    )

    # this creates another short lived helper node. 
    # the spawner asks controller manager to laod the controller, configure it and activate it
    # the spawner exec comes from the contoller manager package, it can spwn different controlelrs depending on the args given to it
    # the name provided here is the name of the helper process not the controller itself
    # the first arg we pass in tells the spawner which controller instace to load, and the 
    # controller manager looks up the supplied name in the YAML
    # '--controller-manager', '/controller_manager' tells ths spawner which controller manager to contact, in our yaml file our controller manager is called controller_manager, so we'll be able to find it
    # the timeour arg, tells us how long to wait for the controller manager to become available
    # after successfully activating the controller, the spawner exits, the cotroller itself will continue running inside the controlller manager.
    joint_state_broadcaster_spawner = Node(
        package='controller_manager',
        executable='spawner',
        name='joint_state_broadcaster_spawner',
        output='screen',
        arguments=[
            'joint_state_broadcaster',
            '--controller-manager', '/controller_manager',
            '--controller-manager-timeout', '60',
        ],
    )

    # creates a short lived helper node same as we do for the broadcast spawner
    joint_trajectory_controller_spawner = Node(
        package='controller_manager',
        executable='spawner',
        name='joint_trajectory_controller_spawner',
        output='screen',
        arguments=[
            'joint_trajectory_controller',
            '--controller-manager', '/controller_manager',
            '--controller-manager-timeout', '60',
        ],
    )

    return LaunchDescription([
        DeclareLaunchArgument(
            'gz_args',
            default_value='-r empty.sdf',
            description=(
                'Gazebo arguments; use "-s -r empty.sdf" for server-only.'
            ),
        ),
        gazebo,
        robot_state_publisher,
        spawn_orion,
        simulator_bridge,
        joint_state_broadcaster_spawner,
        joint_trajectory_controller_spawner,
    ])
