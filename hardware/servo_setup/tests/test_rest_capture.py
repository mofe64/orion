from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import yaml

from orion_servo_setup.pose_execution import load_hardware_calibration
from orion_servo_setup.rest_capture import (
    RestCaptureError,
    positions_to_rest_angles,
    validate_rest_stability,
    write_rest_pose,
)
from test_pose_execution import NEUTRALS, calibration_document, pose_document


OPERATIONAL_RANGES = {name: (-2.0, 2.0) for name in NEUTRALS}


class RestCaptureTests(unittest.TestCase):
    def _calibration(self, directory: str):
        path = Path(directory) / "calibration.json"
        path.write_text(json.dumps(calibration_document()), encoding="utf-8")
        return load_hardware_calibration(path)

    def test_raw_capture_converts_to_radians_and_round_trips_to_steps(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            calibration = self._calibration(directory)
            positions = {name: raw + 100 for name, raw in NEUTRALS.items()}

            angles = positions_to_rest_angles(
                positions, calibration, OPERATIONAL_RANGES
            )

        for angle in angles.values():
            self.assertAlmostEqual(angle, 100 * 2.0 * 3.141592653589793 / 4096, places=7)

    def test_capture_outside_calibrated_range_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            calibration = self._calibration(directory)
            positions = dict(NEUTRALS)
            positions["elbow_pitch_joint"] += 1100

            with self.assertRaisesRegex(RestCaptureError, "outside calibrated"):
                positions_to_rest_angles(positions, calibration, OPERATIONAL_RANGES)

    def test_capture_outside_shared_pose_range_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            calibration = self._calibration(directory)
            positions = dict(NEUTRALS)
            positions["elbow_pitch_joint"] += 100
            ranges = dict(OPERATIONAL_RANGES)
            ranges["elbow_pitch_joint"] = (-0.1, 0.1)

            with self.assertRaisesRegex(RestCaptureError, "shared pose-library"):
                positions_to_rest_angles(positions, calibration, ranges)

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


if __name__ == "__main__":
    unittest.main()
