"""Launch Orion in MuJoCo behind the standard ros2_control interface."""

from pathlib import Path
import xml.etree.ElementTree as ET

from ament_index_python.packages import get_package_share_directory
from launch import LaunchDescription
from launch.actions import DeclareLaunchArgument, OpaqueFunction, Shutdown
from launch.substitutions import LaunchConfiguration
from launch_ros.actions import Node
from launch_ros.parameter_descriptions import ParameterFile


CONTROLLED_JOINTS = (
    "base_yaw_joint",
    "shoulder_pitch_joint",
    "elbow_pitch_joint",
    "head_roll_joint",
    "head_pitch_joint",
)


def build_mujoco_robot_description(
    urdf_path: Path,
    scene_path: Path,
    *,
    headless: bool,
) -> str:
    """Return Orion's URDF with its control backend changed to MuJoCo.

    The checked-in URDF remains the Gazebo description. This function changes
    only the copy passed to the MuJoCo controller manager at launch time.
    """

    root = ET.fromstring(urdf_path.read_text(encoding="utf-8"))
    ros2_control = root.find("ros2_control")
    if ros2_control is None:
        raise ValueError("Orion URDF has no ros2_control section")

    hardware = ros2_control.find("hardware")
    plugin = hardware.find("plugin") if hardware is not None else None
    if hardware is None or plugin is None:
        raise ValueError("Orion ros2_control section has no hardware plugin")

    joint_names = tuple(
        joint.get("name") for joint in ros2_control.findall("joint")
    )
    if joint_names != CONTROLLED_JOINTS:
        raise ValueError(
            "Orion ros2_control joints do not match the canonical joint order"
        )

    ros2_control.set("name", "OrionMujocoSystem")
    plugin.text = "mujoco_ros2_control/MujocoSystemInterface"

    model_parameter = ET.SubElement(hardware, "param", name="mujoco_model")
    model_parameter.text = str(scene_path)
    speed_parameter = ET.SubElement(
        hardware, "param", name="sim_speed_factor"
    )
    speed_parameter.text = "1.0"
    headless_parameter = ET.SubElement(hardware, "param", name="headless")
    headless_parameter.text = "true" if headless else "false"

    # This plugin belongs to Gazebo itself. MuJoCo runs its own control node.
    for gazebo_element in root.findall("gazebo"):
        root.remove(gazebo_element)

    return ET.tostring(root, encoding="unicode")


def launch_setup(context):
    """Resolve launch arguments and create Orion's MuJoCo nodes."""

    package_share = Path(get_package_share_directory("orion_description"))
    controller_config = package_share / "config" / "orion_controllers.yaml"
    urdf_path = package_share / "urdf" / "orion.urdf"
    scene_path = package_share / "mujoco" / "scene.xml"
    headless = LaunchConfiguration("headless").perform(context).lower() in (
        "true",
        "1",
        "yes",
    )

    robot_description = build_mujoco_robot_description(
        urdf_path,
        scene_path,
        headless=headless,
    )

    robot_state_publisher = Node(
        package="robot_state_publisher",
        executable="robot_state_publisher",
        name="robot_state_publisher",
        output="screen",
        parameters=[
            {
                "robot_description": robot_description,
                "use_sim_time": True,
            }
        ],
    )

    control_node = Node(
        package="mujoco_ros2_control",
        executable="ros2_control_node",
        output="screen",
        emulate_tty=True,
        parameters=[
            {"use_sim_time": True},
            ParameterFile(str(controller_config), allow_substs=True),
        ],
        on_exit=Shutdown(),
    )

    controller_spawners = [
        Node(
            package="controller_manager",
            executable="spawner",
            name=f"{controller_name}_spawner",
            output="screen",
            arguments=[
                controller_name,
                "--controller-manager",
                "/controller_manager",
                "--controller-manager-timeout",
                "60",
                "--param-file",
                str(controller_config),
            ],
        )
        for controller_name in (
            "joint_state_broadcaster",
            "joint_trajectory_controller",
        )
    ]

    return [robot_state_publisher, control_node, *controller_spawners]


def generate_launch_description():
    """Describe the ROS-controlled MuJoCo simulation."""

    return LaunchDescription(
        [
            DeclareLaunchArgument(
                "headless",
                default_value="false",
                description="Run MuJoCo without its simulator window.",
            ),
            OpaqueFunction(function=launch_setup),
        ]
    )
