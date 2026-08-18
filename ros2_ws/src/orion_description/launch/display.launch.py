from pathlib import Path
# ament_index is ROS's record of installed packages
from ament_index_python.packages import get_package_share_directory
# launchDescription is a container that holds the actions we want ROS to execute when we run the launch file
from launch import LaunchDescription
from launch_ros.actions import Node


# this is the main function that ROS will execute when we run the launch file
# must return a LaunchDescription object
def generate_launch_description():
    # here we are asking ROS to find the installed share directory for the orion_description package
    # this avoids hardcoding the path to our orion_description
    package_share = Path(get_package_share_directory('orion_description'))
    # build path to our urdf file
    urdf_path = package_share / 'urdf' / 'orion.urdf'
    # build path to our rviz configuration file
    rviz_config_path = package_share / 'rviz' / 'orion.rviz'

    # opens orion.urdf and reads the complete XML file into one Python string
    robot_description = urdf_path.read_text(encoding='utf-8')

    # create a parameter dict for our robot_desc
    # a ros param is a named config value belonging to a node
    # we use this param for our GUI and robot state publisher nodes
    description_parameter = {'robot_description': robot_description}

    # create and return our launch description
    # the nodes are listed in order of execution, but we should not 
    # assume that the first node will be completely ready before the second node starts
    # ros topics and discovery allow nodes to wait for one another
    # the first node is the Joint state publisher gui this allows us to manually 
    # adjust joint states, it used the provided urdf to disocover the joints and their limits
    # it publishes a joint state message on the /joint_states topic
    # the second node is the robot state publisher, this node calculates where every link coordinate frame is located
    # it combines the robot structure from the urdf with the current joint angles from /joint_states
    # and uses that to get the current transform for every link in the robot
    # it publishes 3 important topics
    # - /tf this contains the transforms that can change
    # - /tf_static this contains the transforms that never change
    # - /robot_description this contains the urdf xml, other nodes can sub to this to know what the robot looks like (rviz does this)
    # the third node is rviz2, this is our 3d visualizer
    # its job is to turn the ros info into something we can see, it receives 
    # the transforms from /tf and /tf_static and the robot description from /robot_description
    # and uses that to display the robot in a 3d window
    # we pass it the path to our rviz configuration file so it knows what to display
    return LaunchDescription([
        Node(
            package='joint_state_publisher_gui',
            executable='joint_state_publisher_gui',
            name='joint_state_publisher_gui',
            output='screen',
            parameters=[description_parameter],
        ),
        Node(
            package='robot_state_publisher',
            executable='robot_state_publisher',
            name='robot_state_publisher',
            output='screen',
            parameters=[description_parameter],
        ),
        Node(
            package='rviz2',
            executable='rviz2',
            name='rviz2',
            output='screen',
            arguments=['-d', str(rviz_config_path)],
        ),
    ])
