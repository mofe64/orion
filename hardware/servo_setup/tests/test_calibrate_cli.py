from __future__ import annotations

import io
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

from orion_servo_setup.calibrate_cli import _format_capture_line, main
from orion_servo_setup.calibration import initialize_captures, update_captures
from orion_servo_setup.provisioning import ORION_SERVO_ASSIGNMENTS


class FakeCalibrationBus:
    def __init__(self) -> None:
        self.is_connected = False
        self.disconnect_calls: list[bool] = []
        self.disable_calls: list[tuple[object, int]] = []

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

    def sync_read(self, data_name: str, motors=None, *, normalize: bool = True, num_retry: int = 0):
        return {item.joint_name: 2048 for item in ORION_SERVO_ASSIGNMENTS}


class CalibrateCliTests(unittest.TestCase):
    def test_live_capture_line_fits_a_normal_terminal(self) -> None:
        neutral = {item.joint_name: 2048 for item in ORION_SERVO_ASSIGNMENTS}
        captures = initialize_captures(neutral)
        negative_positions = {
            item.joint_name: 1098 for item in ORION_SERVO_ASSIGNMENTS
        }
        negative_positions["base_yaw_joint"] = 1020
        negative_positions["head_roll_joint"] = 646
        captures = update_captures(captures, negative_positions)
        positive_positions = {
            item.joint_name: 2731 for item in ORION_SERVO_ASSIGNMENTS
        }
        positive_positions["base_yaw_joint"] = 3098
        positive_positions["head_roll_joint"] = 3090
        captures = update_captures(captures, positive_positions)

        line = _format_capture_line(captures)

        self.assertEqual(
            line,
            "1:-1028/+1050 2:-950/+683 3:-950/+683 "
            "4:-1402/+1042 5:-950/+683",
        )
        self.assertLess(len(line), 80)

    def test_dry_run_never_opens_hardware_or_writes_file(self) -> None:
        stream = io.StringIO()
        with (
            patch(
                "orion_servo_setup.calibrate_cli.create_lerobot_bus",
                side_effect=AssertionError("hardware bus must not be created"),
            ),
            redirect_stdout(stream),
        ):
            result = main(["--port", "/dev/not-opened", "--dry-run"])

        self.assertEqual(result, 0)
        self.assertIn("ID 5: head_pitch_joint", stream.getvalue())
        self.assertIn("no serial port was opened", stream.getvalue())

    def test_complete_session_saves_once_and_cleans_up(self) -> None:
        bus = FakeCalibrationBus()
        neutral = {item.joint_name: 2048 for item in ORION_SERVO_ASSIGNMENTS}
        captures = initialize_captures(neutral)
        captures = update_captures(
            captures,
            {item.joint_name: 1448 for item in ORION_SERVO_ASSIGNMENTS},
        )
        captures = update_captures(
            captures,
            {item.joint_name: 2648 for item in ORION_SERVO_ASSIGNMENTS},
        )

        with tempfile.TemporaryDirectory() as directory:
            output_path = Path(directory) / "calibration.json"
            stream = io.StringIO()
            with (
                patch("orion_servo_setup.calibrate_cli.create_lerobot_bus", return_value=bus),
                patch("orion_servo_setup.calibrate_cli._record_until_enter", return_value=captures),
                patch("builtins.input", side_effect=["CALIBRATE ALL", "", ""]),
                redirect_stdout(stream),
            ):
                result = main(
                    ["--port", "/dev/fake", "--output", str(output_path)]
                )

            self.assertEqual(result, 0)
            self.assertTrue(output_path.exists())
            self.assertEqual(bus.disable_calls, [(None, 2)])
            self.assertEqual(bus.disconnect_calls, [True])
            self.assertIn("complete for all five joints", stream.getvalue())


if __name__ == "__main__":
    unittest.main()
