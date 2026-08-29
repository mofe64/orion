from __future__ import annotations

import io
import json
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

from orion_servo_setup.calibration import (
    build_calibration_document,
    initialize_captures,
    update_captures,
)
from orion_servo_setup.provisioning import ORION_SERVO_ASSIGNMENTS
from orion_servo_setup.supported_rest_cli import main


class FakeSupportedRestBus:
    def __init__(self) -> None:
        self.is_connected = False
        self.disable_calls = 0

    def connect(self, handshake: bool = True) -> None:
        self.is_connected = True

    def sync_read(self, data_name, motors=None, *, normalize=True, num_retry=0):
        if data_name != "Present_Position":
            raise KeyError(data_name)
        positions = {
            item.joint_name: 2048 for item in ORION_SERVO_ASSIGNMENTS
        }
        positions["shoulder_pitch_joint"] = 1348
        return positions

    def disable_torque(self, motors=None, num_retry=0) -> None:
        self.disable_calls += 1

    def disconnect(self, disable_torque=True) -> None:
        self.is_connected = False


class SupportedRestCliTests(unittest.TestCase):
    def test_dry_run_does_not_open_hardware(self) -> None:
        stream = io.StringIO()
        with (
            patch(
                "orion_servo_setup.supported_rest_cli.create_lerobot_bus",
                side_effect=AssertionError("hardware must not be opened"),
            ),
            redirect_stdout(stream),
        ):
            result = main(["--port", "/dev/not-opened", "--dry-run"])

        self.assertEqual(result, 0)
        self.assertIn("hardware was not opened", stream.getvalue())

    def test_accepts_live_supported_endpoint_and_backs_up_calibration(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            calibration_path = Path(directory) / "calibration.json"
            neutral = {
                item.joint_name: 2048 for item in ORION_SERVO_ASSIGNMENTS
            }
            captures = initialize_captures(neutral)
            captures = update_captures(
                captures,
                {item.joint_name: 1400 for item in ORION_SERVO_ASSIGNMENTS},
            )
            captures = update_captures(
                captures,
                {item.joint_name: 2700 for item in ORION_SERVO_ASSIGNMENTS},
            )
            document = build_calibration_document(captures, port="/dev/fake")
            calibration_path.write_text(json.dumps(document), encoding="utf-8")
            bus = FakeSupportedRestBus()
            stream = io.StringIO()
            with (
                patch(
                    "orion_servo_setup.supported_rest_cli.create_lerobot_bus",
                    return_value=bus,
                ),
                patch("orion_servo_setup.supported_rest_cli.read_motion_preflight"),
                patch("builtins.input", return_value="ACCEPT SHOULDER REST"),
                redirect_stdout(stream),
            ):
                result = main(
                    [
                        "--port",
                        "/dev/fake",
                        "--calibration",
                        str(calibration_path),
                    ]
                )

            saved = json.loads(calibration_path.read_text(encoding="utf-8"))
            backups = list(Path(directory).glob("calibration.backup-*.json"))

        self.assertEqual(result, 0)
        self.assertEqual(
            saved["joints"]["shoulder_pitch_joint"]["safe_min_delta_raw"],
            -700,
        )
        self.assertEqual(
            saved["joints"]["shoulder_pitch_joint"]["supported_rest_minimum_raw"],
            1348,
        )
        self.assertEqual(len(backups), 1)
        self.assertGreaterEqual(bus.disable_calls, 1)
        self.assertFalse(bus.is_connected)


if __name__ == "__main__":
    unittest.main()
