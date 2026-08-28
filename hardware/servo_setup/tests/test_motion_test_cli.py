from __future__ import annotations

import io
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

from orion_servo_setup.archived.motion_test import MotionResult
from orion_servo_setup.archived.motion_test_cli import main
from orion_servo_setup.provisioning import ORION_SERVO_ASSIGNMENTS


class FakeMotionCliBus:
    def __init__(self) -> None:
        self.is_connected = False
        self.disable_calls: list[tuple[object, int]] = []
        self.disconnect_calls: list[bool] = []

    def connect(self, handshake: bool = True) -> None:
        self.is_connected = True

    def disconnect(self, disable_torque: bool = True) -> None:
        self.disconnect_calls.append(disable_torque)
        self.is_connected = False

    def disable_torque(self, motors=None, num_retry: int = 0) -> None:
        self.disable_calls.append((motors, num_retry))

    def ping(self, motor: str, num_retry: int = 0, raise_on_error: bool = False) -> int:
        return 777

    def read(
        self,
        data_name: str,
        motor: str,
        *,
        normalize: bool = True,
        num_retry: int = 0,
    ) -> int:
        return {
            "Operating_Mode": 0,
            "Torque_Enable": 0,
            "Present_Position": 2048,
            "Present_Voltage": 62,
            "Present_Temperature": 25,
            "Status": 0,
        }[data_name]


class MotionTestCliTests(unittest.TestCase):
    def test_dry_run_does_not_create_hardware_bus(self) -> None:
        stream = io.StringIO()
        with (
            patch(
                "orion_servo_setup.archived.motion_test_cli.create_lerobot_bus",
                side_effect=AssertionError("hardware bus must not be created"),
            ),
            redirect_stdout(stream),
        ):
            result = main(["--port", "/dev/not-opened", "--dry-run"])

        self.assertEqual(result, 0)
        self.assertIn("ID 1: base_yaw_joint", stream.getvalue())
        self.assertIn("Only one joint is enabled at a time", stream.getvalue())
        self.assertIn("no serial port was opened", stream.getvalue())

    def test_wrong_confirmation_never_creates_bus(self) -> None:
        stream = io.StringIO()
        with (
            patch(
                "orion_servo_setup.archived.motion_test_cli.create_lerobot_bus",
                side_effect=AssertionError("hardware bus must not be created"),
            ),
            patch("builtins.input", return_value="no"),
            redirect_stdout(stream),
        ):
            result = main(["--port", "/dev/not-opened"])

        self.assertEqual(result, 2)
        self.assertIn("cancelled", stream.getvalue())

    def test_complete_session_uses_all_five_joints_and_cleans_up_bus(self) -> None:
        bus = FakeMotionCliBus()
        tested_joints: list[str] = []

        def fake_nudge(bus_arg, assignment, *, direction):
            self.assertIs(bus_arg, bus)
            self.assertEqual(direction, 1)
            tested_joints.append(assignment.joint_name)
            return MotionResult(assignment, 2048, 2058, 2058, 130.0, 25, True)

        stream = io.StringIO()
        responses = ["TEST ALL", *("NUDGE +" for _ in ORION_SERVO_ASSIGNMENTS)]
        with (
            patch(
                "orion_servo_setup.archived.motion_test_cli.create_lerobot_bus",
                return_value=bus,
            ),
            patch(
                "orion_servo_setup.archived.motion_test_cli.nudge_joint",
                side_effect=fake_nudge,
            ),
            patch("builtins.input", side_effect=responses),
            redirect_stdout(stream),
        ):
            result = main(["--port", "/dev/fake"])

        self.assertEqual(result, 0)
        self.assertEqual(tested_joints, [item.joint_name for item in ORION_SERVO_ASSIGNMENTS])
        self.assertEqual(bus.disable_calls, [(None, 2)])
        self.assertEqual(bus.disconnect_calls, [True])
        self.assertIn("5 passed, 0 skipped", stream.getvalue())

    def test_start_id_skips_prompts_but_preflights_all_earlier_ids(self) -> None:
        bus = FakeMotionCliBus()
        tested_joints: list[str] = []

        def fake_nudge(bus_arg, assignment, *, direction):
            tested_joints.append(assignment.joint_name)
            return MotionResult(assignment, 2048, 2058, 2052, 130.0, 25, False)

        stream = io.StringIO()
        with (
            patch(
                "orion_servo_setup.archived.motion_test_cli.create_lerobot_bus",
                return_value=bus,
            ),
            patch(
                "orion_servo_setup.archived.motion_test_cli.nudge_joint",
                side_effect=fake_nudge,
            ),
            patch("builtins.input", side_effect=["TEST ALL", "NUDGE +", "NUDGE -"]),
            redirect_stdout(stream),
        ):
            result = main(["--port", "/dev/fake", "--start-id", "4"])

        self.assertEqual(result, 0)
        self.assertEqual(tested_joints, ["head_roll_joint", "head_pitch_joint"])
        self.assertIn("ID 1: base_yaw_joint (preflight only)", stream.getvalue())
        self.assertIn("directional response confirmed", stream.getvalue())


if __name__ == "__main__":
    unittest.main()
