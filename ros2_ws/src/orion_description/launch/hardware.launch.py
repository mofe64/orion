"""Launch physical Orion through its C++ ros2_control hardware plugin."""

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


def build_hardware_robot_description(
    urdf_path: Path,
    *,
    port: str,
    baud_rate: int,
    calibration_file: Path,
) -> str:
    """Return Orion's URDF with the physical STS3215 backend selected."""

    root = ET.fromstring(urdf_path.read_text(encoding="utf-8"))
    ros2_control = root.find("ros2_control")
    if ros2_control is None:
        raise ValueError("Orion URDF has no ros2_control section")

    hardware = ros2_control.find("hardware")
    plugin = hardware.find("plugin") if hardware is not None else None
    if hardware is None or plugin is None:
        raise ValueError("Orion ros2_control section has no hardware plugin")

    joints = ros2_control.findall("joint")
    joint_names = tuple(joint.get("name") for joint in joints)
    if joint_names != CONTROLLED_JOINTS:
        raise ValueError(
            "Orion ros2_control joints do not match the canonical joint order"
        )

    ros2_control.set("name", "OrionSTS3215System")
    plugin.text = "orion_hardware/STS3215System"
    for parameter in hardware.findall("param"):
        hardware.remove(parameter)

    parameters = {
        "port": port,
        "baud_rate": str(baud_rate),
        "calibration_file": str(calibration_file),
    }
    for name, value in parameters.items():
        element = ET.SubElement(hardware, "param", name=name)
        element.text = value

    # The physical driver has no calibrated joint-torque estimate. Do not
    # export Gazebo's simulated effort channel as if it were measured data.
    for joint in joints:
        for state_interface in joint.findall("state_interface"):
            if state_interface.get("name") == "effort":
                joint.remove(state_interface)

    # These plugins and sensors run inside Gazebo, not on the physical robot.
    for gazebo_element in root.findall("gazebo"):
        root.remove(gazebo_element)

    return ET.tostring(root, encoding="unicode")


def launch_setup(context):
    """Resolve physical connection arguments and create ROS 2 nodes."""

    package_share = Path(get_package_share_directory("orion_description"))
    controller_config = package_share / "config" / "orion_controllers.yaml"
    urdf_path = package_share / "urdf" / "orion.urdf"

    port = LaunchConfiguration("port").perform(context)
    calibration_file = Path(
        LaunchConfiguration("calibration_file").perform(context)
    ).expanduser()
    baud_rate = int(LaunchConfiguration("baud_rate").perform(context))
    robot_description = build_hardware_robot_description(
        urdf_path,
        port=port,
        baud_rate=baud_rate,
        calibration_file=calibration_file,
    )

    robot_state_publisher = Node(
        package="robot_state_publisher",
        executable="robot_state_publisher",
        name="robot_state_publisher",
        output="screen",
        parameters=[{"robot_description": robot_description}],
    )

    control_node = Node(
        package="controller_manager",
        executable="ros2_control_node",
        name="controller_manager",
        output="screen",
        emulate_tty=True,
        parameters=[
            {"robot_description": robot_description},
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
    """Describe physical Orion's controller-manager process graph."""

    return LaunchDescription(
        [
            DeclareLaunchArgument(
                "port",
                default_value="/dev/ttyACM0",
                description="Serial device connected to Orion's STS3215 bus.",
            ),
            DeclareLaunchArgument(
                "baud_rate",
                default_value="1000000",
                description="STS3215 bus baud rate.",
            ),
            DeclareLaunchArgument(
                "calibration_file",
                default_value=str(
                    Path.home() / ".config" / "orion" / "servo_calibration.json"
                ),
                description="Orion software calibration JSON file.",
            ),
            OpaqueFunction(function=launch_setup),
        ]
    )
