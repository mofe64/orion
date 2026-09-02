from __future__ import annotations

import io
import json
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

from orion_servo_setup.provisioning import ORION_SERVO_ASSIGNMENTS
from orion_servo_setup.rest_cli import main


NEUTRALS = {
    "base_yaw_joint": 942,
    "shoulder_pitch_joint": 3400,
    "elbow_pitch_joint": 789,
    "head_roll_joint": 2753,
    "head_pitch_joint": 3476,
}


def calibration_document() -> dict[str, object]:
    return {
        "schema_version": 1,
        "robot": "orion",
        "servo_model": "sts3215",
        "writes_servo_eeprom": False,
        "joints": {
            item.joint_name: {
                "servo_id": item.servo_id,
                "neutral_raw": NEUTRALS[item.joint_name],
                "encoder_direction": 1,
                "safe_min_delta_raw": -1004,
                "safe_max_delta_raw": 1004,
            }
            for item in ORION_SERVO_ASSIGNMENTS
        },
    }


class FakeRestBus:
    def __init__(self) -> None:
        self.is_connected = False
        self.disable_calls = 0

    def connect(self, handshake=True):
        self.is_connected = True

    def sync_read(self, data_name, motors=None, *, normalize=True, num_retry=0):
        if data_name != "Present_Position":
            raise KeyError(data_name)
        return dict(NEUTRALS)

    def disable_torque(self, motors=None, num_retry=0):
        self.disable_calls += 1

    def disconnect(self, disable_torque=True):
        self.is_connected = False


class RestCliTests(unittest.TestCase):
    def test_dry_run_does_not_open_hardware_or_write_pose_library(self) -> None:
        stream = io.StringIO()
        with (
            patch(
                "orion_servo_setup.rest_cli.create_lerobot_bus",
                side_effect=AssertionError("hardware bus must not be created"),
            ),
            patch(
                "orion_servo_setup.rest_cli.write_rest_pose",
                side_effect=AssertionError("pose library must not be written"),
            ),
            redirect_stdout(stream),
        ):
            result = main(["--port", "/dev/not-opened", "--dry-run"])

        self.assertEqual(result, 0)
        self.assertIn("Would capture rest", stream.getvalue())

    def test_successful_capture_checks_stability_writes_once_and_cleans_up(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            calibration = Path(directory) / "calibration.json"
            calibration.write_text(json.dumps(calibration_document()), encoding="utf-8")
            bus = FakeRestBus()
            stream = io.StringIO()
            prompts = iter(["", "", "y"])
            with (
                patch("builtins.input", side_effect=lambda _: next(prompts)),
                patch("orion_servo_setup.rest_cli.create_lerobot_bus", return_value=bus),
                patch("orion_servo_setup.rest_cli.read_motion_preflight"),
                patch("orion_servo_setup.rest_cli.time.sleep"),
                patch(
                    "orion_servo_setup.rest_cli.time.monotonic",
                    side_effect=[0.0, 0.0, 5.0],
                ),
                patch("orion_servo_setup.rest_cli.write_rest_pose") as write_pose,
                redirect_stdout(stream),
            ):
                result = main(
                    [
                        "--port",
                        "/dev/fake",
                        "--calibration",
                        str(calibration),
                    ]
                )

        self.assertEqual(result, 0)
        self.assertTrue(write_pose.call_args.kwargs["replace"])
        self.assertGreaterEqual(bus.disable_calls, 1)
        self.assertFalse(bus.is_connected)


if __name__ == "__main__":
    unittest.main()
