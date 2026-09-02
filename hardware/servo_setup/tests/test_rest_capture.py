from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import yaml

from orion_servo_setup.calibration import load_hardware_calibration
from orion_servo_setup.rest_capture import (
    RestCaptureError,
    positions_to_rest_angles,
    validate_rest_stability,
    write_rest_pose,
)
from orion_servo_setup.provisioning import ORION_SERVO_ASSIGNMENTS


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


def pose_document() -> dict[str, object]:
    return {
        "format_version": 2,
        "units": "radians",
        "poses": {
            "rest": {
                "description": "Mechanical rest.",
                "tags": ["shutdown_only", "mechanical_rest"],
                "default_lighting": "off",
                "positions": {name: 0.0 for name in NEUTRALS},
            },
            "home": {
                "description": "Powered home.",
                "tags": ["powered", "idle_anchor"],
                "idle_profile": "home",
                "positions": {name: 0.0 for name in NEUTRALS},
            },
        },
    }


class RestCaptureTests(unittest.TestCase):
    def _calibration(self, directory: str):
        path = Path(directory) / "calibration.json"
        path.write_text(json.dumps(calibration_document()), encoding="utf-8")
        return load_hardware_calibration(path)

    def test_raw_capture_converts_to_radians_and_round_trips_to_steps(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            calibration = self._calibration(directory)
            positions = {name: raw + 100 for name, raw in NEUTRALS.items()}

            angles = positions_to_rest_angles(positions, calibration)

        for angle in angles.values():
            self.assertAlmostEqual(angle, 100 * 2.0 * 3.141592653589793 / 4096, places=7)

    def test_capture_outside_calibrated_range_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            calibration = self._calibration(directory)
            positions = dict(NEUTRALS)
            positions["elbow_pitch_joint"] += 1100

            with self.assertRaisesRegex(RestCaptureError, "outside calibrated"):
                positions_to_rest_angles(positions, calibration)

    def test_stability_rejects_torque_off_drift(self) -> None:
        sample = dict(NEUTRALS)
        sample["shoulder_pitch_joint"] += 11

        with self.assertRaisesRegex(RestCaptureError, "moved with torque off"):
            validate_rest_stability(NEUTRALS, [sample])

    def test_rest_pose_is_added_and_requires_explicit_replace(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "poses.yaml"
            document = pose_document()
            del document["poses"]["rest"]
            path.write_text(yaml.safe_dump(document, sort_keys=False), encoding="utf-8")
            angles = {name: 0.0 for name in NEUTRALS}

            write_rest_pose(path, angles)
            saved = yaml.safe_load(path.read_text(encoding="utf-8"))
            self.assertEqual(saved["poses"]["rest"]["positions"], angles)
            with self.assertRaisesRegex(RestCaptureError, "--replace"):
                write_rest_pose(path, angles)

    def test_replacing_rest_preserves_unrelated_yaml_formatting(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "poses.yaml"
            path.write_text(
                "format_version: 2\n"
                "units: radians\n\n"
                "poses:\n"
                "  rest:\n"
                "    description: old\n"
                "    positions:\n"
                "      base_yaw_joint: 0.0\n\n"
                "  home:\n"
                "    description: Keep this formatting.\n"
                "    positions:\n"
                "      base_yaw_joint: -0.30  # unchanged\n",
                encoding="utf-8",
            )
            angles = {name: 0.0 for name in NEUTRALS}

            write_rest_pose(path, angles, replace=True)
            saved_text = path.read_text(encoding="utf-8")

        self.assertIn("      base_yaw_joint: -0.30  # unchanged\n", saved_text)
        self.assertIn("    description: Keep this formatting.\n", saved_text)


if __name__ == "__main__":
    unittest.main()
