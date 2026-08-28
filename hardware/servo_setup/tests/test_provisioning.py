from __future__ import annotations

import unittest

from orion_servo_setup.provisioning import (
    ORION_SERVO_ASSIGNMENTS,
    ProvisioningCancelled,
    ServoAssignment,
    provision_servos,
    provisioning_plan,
    validate_assignments,
)


class FakeBus:
    def __init__(self) -> None:
        self.setup_calls: list[str] = []

    def setup_motor(self, motor: str) -> None:
        self.setup_calls.append(motor)


class ProvisioningTests(unittest.TestCase):
    def test_orion_mapping_matches_reference_ids_and_semantic_joint_names(self) -> None:
        self.assertEqual(
            [
                (item.joint_name, item.joint_ref_name, item.servo_id)
                for item in ORION_SERVO_ASSIGNMENTS
            ],
            [
                ("base_yaw_joint", "base_yaw", 1),
                ("shoulder_pitch_joint", "base_pitch", 2),
                ("elbow_pitch_joint", "elbow_pitch", 3),
                ("head_roll_joint", "wrist_roll", 4),
                ("head_pitch_joint", "wrist_pitch", 5),
            ],
        )

    def test_default_plan_programs_factory_default_id_one_last(self) -> None:
        self.assertEqual([item.servo_id for item in provisioning_plan()], [5, 4, 3, 2, 1])

    def test_selected_joint_limits_plan_to_one_servo(self) -> None:
        self.assertEqual(
            provisioning_plan(selected_joint="elbow_pitch_joint"),
            (ServoAssignment("elbow_pitch_joint", 3, "elbow_pitch"),),
        )

    def test_provisioning_requires_exact_confirmation_before_every_write(self) -> None:
        bus = FakeBus()
        replies = iter(["PROGRAM", "no"])

        with self.assertRaisesRegex(ProvisioningCancelled, "head_roll_joint"):
            provision_servos(
                bus,
                provisioning_plan(),
                confirm=lambda _: next(replies),
                output=lambda _: None,
            )

        self.assertEqual(bus.setup_calls, ["head_pitch_joint"])

    def test_provisioning_programs_each_confirmed_servo_in_plan_order(self) -> None:
        bus = FakeBus()
        plan = provisioning_plan()

        completed = provision_servos(
            bus,
            plan,
            confirm=lambda _: "PROGRAM",
            output=lambda _: None,
        )

        self.assertEqual(completed, plan)
        self.assertEqual(bus.setup_calls, [item.joint_name for item in plan])

    def test_invalid_assignment_maps_are_rejected(self) -> None:
        cases = [
            ((), "At least one"),
            (
                (ServoAssignment("joint_a", 1, "a"), ServoAssignment("joint_b", 1, "b")),
                "IDs must be unique",
            ),
            ((ServoAssignment("joint_a", 253, "a"),), "between 1 and 252"),
        ]
        for assignments, message in cases:
            with self.subTest(message=message):
                with self.assertRaisesRegex(ValueError, message):
                    validate_assignments(assignments)


if __name__ == "__main__":
    unittest.main()
