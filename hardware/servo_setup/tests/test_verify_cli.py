from __future__ import annotations

import io
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

from orion_servo_setup.verify_cli import main
from orion_servo_setup.verification import AUDIT_REGISTERS


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
        overrides = {
            "Firmware_Major_Version": 2,
            "Firmware_Minor_Version": 54,
            "Present_Position": 1024,
            "Present_Velocity": 3,
            "Present_Load": 12,
            "Present_Voltage": 60,
            "Present_Temperature": 25,
            "Present_Current": 8,
            "Torque_Enable": 0,
            "P_Coefficient": 16,
            "I_Coefficient": 0,
            "D_Coefficient": 32,
            "Acceleration": 254,
            "Goal_Velocity": 0,
            "Torque_Limit": 1000,
        }
        if data_name not in AUDIT_REGISTERS:
            raise AssertionError(f"unexpected register read: {data_name}")
        return overrides.get(data_name, 1)


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
        self.assertIn("bus IDs 1, 2, 3, 4, 5", stream.getvalue())

    def test_successful_verification_disconnects_without_writing_torque(self) -> None:
        bus = FakeVerificationBus()
        stream = io.StringIO()
        with (
            patch("orion_servo_setup.verify_cli.create_lerobot_bus", return_value=bus),
            patch("builtins.input", side_effect=AssertionError("CLI must not prompt")),
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
        self.assertEqual(bus.connect_calls, [False])
        self.assertEqual(bus.disconnect_calls, [False])
        output = stream.getvalue()
        self.assertIn("Orion STS3215 audit: /dev/fake", output)
        self.assertIn("head_pitch_joint", output)
        self.assertIn("P_Coefficient", output)
        self.assertIn("Goal_Velocity", output)
        self.assertIn("Torque_Limit", output)


if __name__ == "__main__":
    unittest.main()
