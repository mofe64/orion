from __future__ import annotations

import unittest

from orion_servo_setup.provisioning import ServoAssignment
from orion_servo_setup.verification import read_servo_telemetry, verification_plan


class FakeReadOnlyBus:
    def __init__(self) -> None:
        self.calls: list[tuple[object, ...]] = []

    def ping(self, motor: str, num_retry: int = 0, raise_on_error: bool = False) -> int:
        self.calls.append(("ping", motor, num_retry, raise_on_error))
        return 777

    def read(
        self,
        data_name: str,
        motor: str,
        *,
        normalize: bool = True,
        num_retry: int = 0,
    ) -> int:
        self.calls.append(("read", data_name, motor, normalize, num_retry))
        values = {
            "Present_Position": 2048,
            "Present_Voltage": 60,
            "Present_Temperature": 24,
            "Torque_Enable": 0,
        }
        return values[data_name]


class VerificationTests(unittest.TestCase):
    def test_plan_uses_ascending_bus_id_order(self) -> None:
        self.assertEqual([item.servo_id for item in verification_plan()], [1, 2, 3, 4, 5])

    def test_plan_can_select_one_joint(self) -> None:
        plan = verification_plan(selected_joint="head_roll_joint")
        self.assertEqual(plan, (ServoAssignment("head_roll_joint", 4, "wrist_roll"),))

    def test_telemetry_uses_only_ping_and_read_operations(self) -> None:
        bus = FakeReadOnlyBus()
        assignment = ServoAssignment("head_pitch_joint", 5, "wrist_pitch")

        snapshots = read_servo_telemetry(bus, (assignment,))

        self.assertEqual(len(snapshots), 1)
        snapshot = snapshots[0]
        self.assertEqual(snapshot.model_number, 777)
        self.assertEqual(snapshot.position_raw, 2048)
        self.assertEqual(snapshot.voltage_v, 6.0)
        self.assertEqual(snapshot.temperature_c, 24)
        self.assertFalse(snapshot.torque_enabled)
        self.assertEqual(
            bus.calls,
            [
                ("ping", "head_pitch_joint", 2, True),
                ("read", "Present_Position", "head_pitch_joint", False, 2),
                ("read", "Present_Voltage", "head_pitch_joint", False, 2),
                ("read", "Present_Temperature", "head_pitch_joint", False, 2),
                ("read", "Torque_Enable", "head_pitch_joint", False, 2),
            ],
        )


if __name__ == "__main__":
    unittest.main()
