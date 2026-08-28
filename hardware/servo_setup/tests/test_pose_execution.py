from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import yaml

from orion_servo_setup.pose_execution import (
    PoseExecutionError,
    build_hardware_pose_plan,
    execute_pose_cycle,
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


def pose_document(**overrides: float) -> dict[str, object]:
    positions = {
        "base_yaw_joint": -0.30,
        "shoulder_pitch_joint": 0.30,
        "elbow_pitch_joint": 0.10,
        "head_roll_joint": -0.45,
        "head_pitch_joint": 0.08,
    }
    positions.update(overrides)
    return {
        "format_version": 1,
        "units": "radians",
        "poses": {
            "home": {"positions": positions},
            "rest": {
                "positions": {
                    "base_yaw_joint": 0.0,
                    "shoulder_pitch_joint": 0.1,
                    "elbow_pitch_joint": -0.1,
                    "head_roll_joint": 0.0,
                    "head_pitch_joint": 0.1,
                }
            },
        },
    }


class FakePoseBus:
    def __init__(self, initial_offset: int = 0) -> None:
        self.positions = {name: value + initial_offset for name, value in NEUTRALS.items()}
        self.torque_enabled = False
        self.calls: list[tuple[object, ...]] = []
        self.current_raw = 10

    def read(self, data_name, motor, *, normalize=True, num_retry=0):
        raise AssertionError("pose execution uses synchronized telemetry")

    def write(self, data_name, motor, value, *, normalize=True, num_retry=0):
        self.calls.append(("write", data_name, motor, value, normalize, num_retry))

    def sync_read(self, data_name, motors=None, *, normalize=True, num_retry=0):
        self.calls.append(("sync_read", data_name, normalize, num_retry))
        if data_name == "Present_Position":
            return dict(self.positions)
        if data_name == "Present_Current":
            return {name: self.current_raw for name in self.positions}
        if data_name == "Present_Temperature":
            return {name: 25 for name in self.positions}
        if data_name == "Status":
            return {name: 0 for name in self.positions}
        raise KeyError(data_name)

    def sync_write(self, data_name, values, *, normalize=True, num_retry=0):
        self.calls.append(("sync_write", data_name, dict(values), normalize, num_retry))
        if data_name == "Goal_Position" and self.torque_enabled:
            self.positions.update({name: int(value) for name, value in values.items()})

    def enable_torque(self, motors=None, num_retry=0):
        self.calls.append(("enable_torque", motors, num_retry))
        self.torque_enabled = True

    def disable_torque(self, motors=None, num_retry=0):
        self.calls.append(("disable_torque", motors, num_retry))
        self.torque_enabled = False


class PoseExecutionTests(unittest.TestCase):
    def _files(self, directory: str, pose_data=None) -> tuple[Path, Path]:
        calibration = Path(directory) / "calibration.json"
        poses = Path(directory) / "poses.yaml"
        calibration.write_text(json.dumps(calibration_document()), encoding="utf-8")
        poses.write_text(
            yaml.safe_dump(pose_data or pose_document()),
            encoding="utf-8",
        )
        return calibration, poses

    def test_named_pose_maps_orion_radians_to_calibrated_raw_targets(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            calibration, poses = self._files(directory)

            plan = build_hardware_pose_plan(
                "home", pose_path=poses, calibration_path=calibration
            )

        self.assertEqual(
            plan.target_positions,
            {
                "base_yaw_joint": 746,
                "shoulder_pitch_joint": 3596,
                "elbow_pitch_joint": 854,
                "head_roll_joint": 2460,
                "head_pitch_joint": 3528,
            },
        )

    def test_pose_outside_measured_hardware_range_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            calibration, poses = self._files(
                directory, pose_document(base_yaw_joint=2.0)
            )

            with self.assertRaisesRegex(PoseExecutionError, "outside calibrated"):
                build_hardware_pose_plan(
                    "home", pose_path=poses, calibration_path=calibration
                )

    def test_pose_cycle_visits_pose_returns_to_rest_and_disables(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            calibration, poses = self._files(directory)
            plan = build_hardware_pose_plan(
                "home", pose_path=poses, calibration_path=calibration
            )
            rest_plan = build_hardware_pose_plan(
                "rest", pose_path=poses, calibration_path=calibration
            )
        bus = FakePoseBus(initial_offset=10)

        result = execute_pose_cycle(
            bus,
            plan,
            rest_plan,
            pose_duration=4.0,
            hold_seconds=0.0,
            return_duration=4.0,
            sleep=lambda _: None,
            should_power_down=lambda: True,
        )

        self.assertEqual(result.pose_name, "home")
        self.assertEqual(result.rest_hold_exit_reason, "power_down_confirmed")
        self.assertEqual(bus.positions, rest_plan.target_positions)
        self.assertFalse(bus.torque_enabled)
        self.assertIn(("enable_torque", None, 2), bus.calls)
        self.assertIn(("disable_torque", None, 2), bus.calls)

    def test_pose_cycle_accepts_any_start_inside_calibrated_range(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            calibration, poses = self._files(directory)
            plan = build_hardware_pose_plan(
                "home", pose_path=poses, calibration_path=calibration
            )
            rest_plan = build_hardware_pose_plan(
                "rest", pose_path=poses, calibration_path=calibration
            )
        bus = FakePoseBus(initial_offset=200)

        execute_pose_cycle(
            bus,
            plan,
            rest_plan,
            pose_duration=4.0,
            hold_seconds=0.0,
            return_duration=4.0,
            sleep=lambda _: None,
            should_power_down=lambda: True,
        )

        self.assertIn(("enable_torque", None, 2), bus.calls)

    def test_pose_cycle_rejects_start_outside_calibrated_range(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            calibration, poses = self._files(directory)
            plan = build_hardware_pose_plan(
                "home", pose_path=poses, calibration_path=calibration
            )
            rest_plan = build_hardware_pose_plan(
                "rest", pose_path=poses, calibration_path=calibration
            )
        bus = FakePoseBus(initial_offset=1100)

        with self.assertRaisesRegex(PoseExecutionError, "outside its calibrated"):
            execute_pose_cycle(
                bus,
                plan,
                rest_plan,
                pose_duration=4.0,
                hold_seconds=0.0,
                return_duration=4.0,
                sleep=lambda _: None,
                should_power_down=lambda: True,
            )

        self.assertNotIn(("enable_torque", None, 2), bus.calls)

    def test_current_fault_disables_all_torque(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            calibration, poses = self._files(directory)
            plan = build_hardware_pose_plan(
                "home", pose_path=poses, calibration_path=calibration
            )
            rest_plan = build_hardware_pose_plan(
                "rest", pose_path=poses, calibration_path=calibration
            )
        bus = FakePoseBus()
        bus.current_raw = 200

        with self.assertRaisesRegex(PoseExecutionError, "1.0 A"):
            execute_pose_cycle(
                bus,
                plan,
                rest_plan,
                pose_duration=4.0,
                hold_seconds=0.0,
                return_duration=4.0,
                sleep=lambda _: None,
                should_power_down=lambda: True,
            )

        self.assertFalse(bus.torque_enabled)
        self.assertIn(("disable_torque", None, 2), bus.calls)

    def test_ctrl_c_at_rest_disables_torque_without_restarting_motion(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            calibration, poses = self._files(directory)
            plan = build_hardware_pose_plan(
                "home", pose_path=poses, calibration_path=calibration
            )
            rest_plan = build_hardware_pose_plan(
                "rest", pose_path=poses, calibration_path=calibration
            )
        bus = FakePoseBus()

        def interrupt_at_rest() -> bool:
            raise KeyboardInterrupt

        result = execute_pose_cycle(
            bus,
            plan,
            rest_plan,
            pose_duration=4.0,
            hold_seconds=0.0,
            return_duration=4.0,
            sleep=lambda _: None,
            should_power_down=interrupt_at_rest,
        )

        self.assertEqual(result.rest_hold_exit_reason, "interrupt_at_rest")
        self.assertEqual(bus.positions, rest_plan.target_positions)
        self.assertFalse(bus.torque_enabled)


if __name__ == "__main__":
    unittest.main()
