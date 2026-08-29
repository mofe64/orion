from __future__ import annotations

import tempfile
import unittest
from datetime import UTC, datetime
from pathlib import Path

from orion_servo_setup.calibration import (
    CalibrationError,
    build_calibration_document,
    circular_delta,
    initialize_captures,
    update_captures,
    validate_captures,
    write_calibration_file,
)
from orion_servo_setup.provisioning import ORION_SERVO_ASSIGNMENTS


class CalibrationTests(unittest.TestCase):
    def test_circular_delta_handles_encoder_wraparound(self) -> None:
        self.assertEqual(circular_delta(10, 4090), 16)
        self.assertEqual(circular_delta(4090, 10), -16)
        self.assertEqual(circular_delta(100, 3650), 546)

    def test_range_capture_tracks_all_joints_relative_to_neutral(self) -> None:
        neutral = {item.joint_name: 3650 for item in ORION_SERVO_ASSIGNMENTS}
        captures = initialize_captures(neutral)
        captures = update_captures(
            captures,
            {item.joint_name: 3150 for item in ORION_SERVO_ASSIGNMENTS},
        )
        captures = update_captures(
            captures,
            {item.joint_name: 100 for item in ORION_SERVO_ASSIGNMENTS},
        )

        self.assertTrue(all(item.measured_min_delta_raw == -500 for item in captures.values()))
        self.assertTrue(all(item.measured_max_delta_raw == 546 for item in captures.values()))
        validate_captures(captures)

    def test_document_caps_yaw_instead_of_rejecting_measured_overrun(self) -> None:
        neutral = {item.joint_name: 2048 for item in ORION_SERVO_ASSIGNMENTS}
        captures = initialize_captures(neutral)
        negative_positions = {
            item.joint_name: 1400 for item in ORION_SERVO_ASSIGNMENTS
        }
        negative_positions["head_roll_joint"] = 646
        captures = update_captures(
            captures,
            negative_positions,
        )
        positions = {item.joint_name: 2700 for item in ORION_SERVO_ASSIGNMENTS}
        positions["base_yaw_joint"] = 3200
        positions["head_roll_joint"] = 3090
        captures = update_captures(captures, positions)

        document = build_calibration_document(captures, port="/dev/fake")
        base_yaw = document["joints"]["base_yaw_joint"]  # type: ignore[index]
        head_roll = document["joints"]["head_roll_joint"]  # type: ignore[index]

        self.assertEqual(base_yaw["measured_max_delta_raw"], 1152)
        self.assertEqual(base_yaw["safe_max_delta_raw"], 1004)
        self.assertTrue(base_yaw["safety_cap_applied"])
        self.assertEqual(head_roll["measured_min_delta_raw"], -1402)
        self.assertEqual(head_roll["measured_max_delta_raw"], 1042)
        self.assertEqual(head_roll["safe_min_delta_raw"], -1004)
        self.assertEqual(head_roll["safe_max_delta_raw"], 1004)
        self.assertTrue(head_roll["safety_cap_applied"])

    def test_validation_still_rejects_wide_uncapped_joint(self) -> None:
        neutral = {item.joint_name: 2048 for item in ORION_SERVO_ASSIGNMENTS}
        captures = initialize_captures(neutral)
        negative_positions = {
            item.joint_name: 1400 for item in ORION_SERVO_ASSIGNMENTS
        }
        negative_positions["elbow_pitch_joint"] = 646
        captures = update_captures(captures, negative_positions)
        positive_positions = {
            item.joint_name: 2700 for item in ORION_SERVO_ASSIGNMENTS
        }
        positive_positions["elbow_pitch_joint"] = 3090
        captures = update_captures(captures, positive_positions)

        with self.assertRaisesRegex(CalibrationError, "elbow_pitch_joint covered 2444"):
            validate_captures(captures)

    def test_validation_requires_all_five_canonical_joints(self) -> None:
        neutral = {item.joint_name: 2048 for item in ORION_SERVO_ASSIGNMENTS}
        captures = initialize_captures(neutral)
        captures.pop("head_pitch_joint")

        with self.assertRaisesRegex(CalibrationError, "missing: head_pitch_joint"):
            validate_captures(captures)

    def test_validation_requires_both_sides_of_neutral(self) -> None:
        neutral = {item.joint_name: 2048 for item in ORION_SERVO_ASSIGNMENTS}
        captures = initialize_captures(neutral)
        captures = update_captures(
            captures,
            {item.joint_name: 2700 for item in ORION_SERVO_ASSIGNMENTS},
        )

        with self.assertRaisesRegex(CalibrationError, "both sides of neutral"):
            validate_captures(captures)

    def test_document_contains_safe_and_lerobot_compatible_values(self) -> None:
        neutral = {item.joint_name: 3650 for item in ORION_SERVO_ASSIGNMENTS}
        captures = initialize_captures(neutral)
        captures = update_captures(
            captures,
            {item.joint_name: 3050 for item in ORION_SERVO_ASSIGNMENTS},
        )
        captures = update_captures(
            captures,
            {item.joint_name: 200 for item in ORION_SERVO_ASSIGNMENTS},
        )

        document = build_calibration_document(
            captures,
            port="/dev/fake",
            captured_at=datetime(2026, 8, 28, tzinfo=UTC),
        )
        joint = document["joints"]["head_pitch_joint"]  # type: ignore[index]

        self.assertEqual(document["schema_version"], 1)
        self.assertFalse(document["writes_servo_eeprom"])
        self.assertEqual(joint["neutral_raw"], 3650)
        self.assertEqual(joint["safe_min_delta_raw"], -580)
        self.assertEqual(joint["safe_max_delta_raw"], 626)
        self.assertEqual(joint["lerobot_homing_offset"], 1603)
        self.assertEqual(joint["lerobot_safe_range_min"], 1467)
        self.assertEqual(joint["lerobot_safe_range_max"], 2673)

    def test_write_keeps_backup_of_existing_calibration(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "servo_calibration.json"
            output.write_text("old\n", encoding="utf-8")

            backup = write_calibration_file({"schema_version": 1}, output)

            self.assertIsNotNone(backup)
            self.assertEqual(backup.read_text(encoding="utf-8"), "old\n")
            self.assertIn('"schema_version": 1', output.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
