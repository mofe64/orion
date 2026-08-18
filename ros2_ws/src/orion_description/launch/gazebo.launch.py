from pathlib import Path

from ament_index_python.packages import get_package_share_directory
from launch import LaunchDescription
from launch.actions import IncludeLaunchDescription
from launch.launch_description_sources import PythonLaunchDescriptionSource
from launch_ros.actions import Node


def generate_launch_description():
    package_share = Path(get_package_share_directory('orion_description'))
    ros_gz_sim_share = Path(get_package_share_directory('ros_gz_sim'))

    urdf_path = package_share / 'urdf' / 'orion.urdf'
    gazebo_launch_path = ros_gz_sim_share / 'launch' / 'gz_sim.launch.py'

    robot_description = urdf_path.read_text(encoding='utf-8').replace(
        '$(find orion_description)',
        str(package_share),
    )

    gazebo = IncludeLaunchDescription(
        PythonLaunchDescriptionSource(str(gazebo_launch_path)),
        launch_arguments={'gz_args': '-r empty.sdf'}.items(),
    )

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

    clock_bridge = Node(
        package='ros_gz_bridge',
        executable='parameter_bridge',
        name='clock_bridge',
        output='screen',
        arguments=['/clock@rosgraph_msgs/msg/Clock[gz.msgs.Clock'],
    )

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
        gazebo,
        robot_state_publisher,
        spawn_orion,
        clock_bridge,
        joint_state_broadcaster_spawner,
        joint_trajectory_controller_spawner,
    ])
