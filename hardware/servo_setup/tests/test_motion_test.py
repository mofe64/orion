from __future__ import annotations

import unittest

from orion_servo_setup.motion_test import (
    MotionTestError,
    nudge_joint,
    read_motion_preflight,
)
from orion_servo_setup.provisioning import ServoAssignment


class FakeMotionBus:
    def __init__(self) -> None:
        self.values = {
            "Operating_Mode": 0,
            "Torque_Enable": 0,
            "Present_Position": 2048,
            "Present_Voltage": 62,
            "Present_Temperature": 25,
            "Present_Current": 20,
            "Status": 0,
        }
        self.calls: list[tuple[object, ...]] = []

    def ping(self, motor: str, num_retry: int = 0, raise_on_error: bool = False) -> int:
        self.calls.append(("ping", motor, num_retry, raise_on_error))
        return 777

    def read(
        self,
        data_name: str,
        motor: str,
        *,
        normalize: bool = True,
        num_retry: int = 0,
    ) -> int:
        self.calls.append(("read", data_name, motor, normalize, num_retry))
        return self.values[data_name]

    def write(
        self,
        data_name: str,
        motor: str,
        value: int,
        *,
        normalize: bool = True,
        num_retry: int = 0,
    ) -> None:
        self.calls.append(("write", data_name, motor, value, normalize, num_retry))
        if data_name == "Goal_Position" and self.values["Torque_Enable"]:
            self.values["Present_Position"] = value

    def enable_torque(self, motors=None, num_retry: int = 0) -> None:
        self.calls.append(("enable_torque", motors, num_retry))
        self.values["Torque_Enable"] = 1

    def disable_torque(self, motors=None, num_retry: int = 0) -> None:
        self.calls.append(("disable_torque", motors, num_retry))
        self.values["Torque_Enable"] = 0


class MotionTestTests(unittest.TestCase):
    def setUp(self) -> None:
        self.assignment = ServoAssignment("base_yaw_joint", 1, "base_yaw")

    def test_preflight_requires_torque_off(self) -> None:
        bus = FakeMotionBus()
        bus.values["Torque_Enable"] = 1

        with self.assertRaisesRegex(MotionTestError, "already has torque enabled"):
            read_motion_preflight(bus, (self.assignment,))

    def test_nudge_sets_current_goal_before_enabling_and_always_disables(self) -> None:
        bus = FakeMotionBus()

        result = nudge_joint(bus, self.assignment, direction=1, sleep=lambda _: None)

        self.assertEqual(result.start_position_raw, 2048)
        self.assertEqual(result.target_position_raw, 2058)
        self.assertEqual(result.final_position_raw, 2058)
        initial_goal = ("write", "Goal_Position", "base_yaw_joint", 2048, False, 2)
        enable = ("enable_torque", "base_yaw_joint", 2)
        target_goal = ("write", "Goal_Position", "base_yaw_joint", 2058, False, 2)
        disable = ("disable_torque", "base_yaw_joint", 2)
        self.assertLess(bus.calls.index(initial_goal), bus.calls.index(enable))
        self.assertLess(bus.calls.index(enable), bus.calls.index(target_goal))
        self.assertLess(bus.calls.index(target_goal), bus.calls.index(disable))
        self.assertEqual(bus.values["Torque_Enable"], 0)

    def test_nudge_disables_torque_when_current_guard_trips(self) -> None:
        bus = FakeMotionBus()
        bus.values["Present_Current"] = 200

        with self.assertRaisesRegex(MotionTestError, "1.0 A"):
            nudge_joint(bus, self.assignment, direction=1, sleep=lambda _: None)

        self.assertIn(("disable_torque", "base_yaw_joint", 2), bus.calls)
        self.assertEqual(bus.values["Torque_Enable"], 0)


if __name__ == "__main__":
    unittest.main()
