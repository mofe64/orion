"""Keep ROS controller enforcement aligned with Orion execution policy."""

from pathlib import Path

from orion_motion.motion_loader import load_yaml_file
from orion_motion.ros_motion_player import execution_policy_from_data


PACKAGE_DIRECTORY = Path(__file__).parent.parent
CONTROLLER_CONFIG = (
    PACKAGE_DIRECTORY.parent
    / "orion_description"
    / "config"
    / "orion_controllers.yaml"
)


def test_controller_tolerances_match_motion_execution_policy():
    limits = load_yaml_file(PACKAGE_DIRECTORY / "config" / "motion_limits.yaml")
    policy = execution_policy_from_data(
        load_yaml_file(PACKAGE_DIRECTORY / "config" / "execution_policy.yaml")
    )
    controller_file = load_yaml_file(CONTROLLER_CONFIG)
    parameters = controller_file["joint_trajectory_controller"][
        "ros__parameters"
    ]
    constraints = parameters["constraints"]

    assert constraints["goal_time"] == policy.goal_time_tolerance
    assert (
        constraints["stopped_velocity_tolerance"]
        == policy.stopped_velocity_tolerance
    )
    for joint_name in limits["joint_order"]:
        assert (
            constraints[joint_name]["trajectory"]
            == policy.path_position_tolerance
        )
        assert (
            constraints[joint_name]["goal"]
            == policy.goal_position_tolerance
        )
