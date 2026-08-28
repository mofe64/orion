from __future__ import annotations

import io
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

from orion_servo_setup.verify_cli import main


class FakeVerificationBus:
    def __init__(self) -> None:
        self.is_connected = False
        self.connect_calls: list[bool] = []
        self.disconnect_calls: list[bool] = []

    def connect(self, handshake: bool = True) -> None:
        self.connect_calls.append(handshake)
        self.is_connected = True

    def disconnect(self, disable_torque: bool = True) -> None:
        self.disconnect_calls.append(disable_torque)
        self.is_connected = False

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
            "Present_Position": 1024,
            "Present_Voltage": 60,
            "Present_Temperature": 25,
            "Torque_Enable": 0,
        }[data_name]


class VerifyCliTests(unittest.TestCase):
    def test_dry_run_does_not_create_hardware_bus(self) -> None:
        stream = io.StringIO()
        with (
            patch(
                "orion_servo_setup.verify_cli.create_lerobot_bus",
                side_effect=AssertionError("hardware bus must not be created"),
            ),
            redirect_stdout(stream),
        ):
            result = main(["--port", "/dev/not-opened", "--dry-run"])

        self.assertEqual(result, 0)
        self.assertIn("ID 1: base_yaw_joint", stream.getvalue())
        self.assertIn("no serial port was opened", stream.getvalue())

    def test_successful_verification_disconnects_without_writing_torque(self) -> None:
        bus = FakeVerificationBus()
        stream = io.StringIO()
        with (
            patch("orion_servo_setup.verify_cli.create_lerobot_bus", return_value=bus),
            patch("builtins.input", return_value="VERIFY"),
            redirect_stdout(stream),
        ):
            result = main(
                [
                    "--port",
                    "/dev/fake",
                    "--joint",
                    "head_pitch_joint",
                ]
            )

        self.assertEqual(result, 0)
        self.assertEqual(bus.connect_calls, [True])
        self.assertEqual(bus.disconnect_calls, [False])
        self.assertIn("Read-only verification passed", stream.getvalue())


if __name__ == "__main__":
    unittest.main()
