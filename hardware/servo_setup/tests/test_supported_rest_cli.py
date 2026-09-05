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
    def __init__(self, shoulder_raw: int = 1348, elbow_raw: int = 2048) -> None:
        self.is_connected = False
        self.disable_calls = 0
        self.shoulder_raw = shoulder_raw
        self.elbow_raw = elbow_raw

    def connect(self, handshake: bool = True) -> None:
        self.is_connected = True

    def sync_read(self, data_name, motors=None, *, normalize=True, num_retry=0):
        if data_name != "Present_Position":
            raise KeyError(data_name)
        positions = {
            item.joint_name: 2048 for item in ORION_SERVO_ASSIGNMENTS
        }
        positions["shoulder_pitch_joint"] = self.shoulder_raw
        positions["elbow_pitch_joint"] = self.elbow_raw
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
        self.assertIn("Would read shoulder_pitch_joint supported rest", stream.getvalue())

    def test_accepts_live_supported_elbow_maximum(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            calibration_path = Path(directory) / "calibration.json"
            neutral = {item.joint_name: 2048 for item in ORION_SERVO_ASSIGNMENTS}
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
            bus = FakeSupportedRestBus(elbow_raw=2710)
            with (
                patch(
                    "orion_servo_setup.supported_rest_cli.create_lerobot_bus",
                    return_value=bus,
                ),
                patch("orion_servo_setup.supported_rest_cli.read_preflight"),
                patch("builtins.input", return_value="y"),
                redirect_stdout(io.StringIO()),
            ):
                result = main(
                    [
                        "--port",
                        "/dev/fake",
                        "--joint",
                        "elbow_pitch_joint",
                        "--calibration",
                        str(calibration_path),
                    ]
                )

            saved = json.loads(calibration_path.read_text(encoding="utf-8"))

        self.assertEqual(result, 0)
        elbow = saved["joints"]["elbow_pitch_joint"]
        self.assertEqual(elbow["safe_max_delta_raw"], 662)
        self.assertEqual(elbow["supported_rest_maximum_raw"], 2710)

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
                patch("orion_servo_setup.supported_rest_cli.read_preflight"),
                patch("builtins.input", return_value="y"),
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

    def test_position_already_inside_range_is_a_successful_no_op(self) -> None:
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
            original = json.dumps(document)
            calibration_path.write_text(original, encoding="utf-8")
            bus = FakeSupportedRestBus(shoulder_raw=1500)
            stream = io.StringIO()
            with (
                patch(
                    "orion_servo_setup.supported_rest_cli.create_lerobot_bus",
                    return_value=bus,
                ),
                patch("orion_servo_setup.supported_rest_cli.read_preflight"),
                patch(
                    "builtins.input",
                    side_effect=AssertionError("no confirmation should be requested"),
                ),
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

            saved = calibration_path.read_text(encoding="utf-8")

        self.assertEqual(result, 0)
        self.assertEqual(saved, original)
        self.assertIn("Already allowed:", stream.getvalue())


if __name__ == "__main__":
    unittest.main()
