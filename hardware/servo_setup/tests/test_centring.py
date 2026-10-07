from __future__ import annotations

import io
import json
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

from orion_servo_setup.calibration import CalibrationError, crosses_encoder_boundary
from orion_servo_setup.centre_cli import main
from orion_servo_setup.centring import centred_document, plan_centring


# The V2 calibration captured on 2026-10-07: four of five ranges cross 0/4095.
V2_JOINTS = {
    "base_yaw_joint": (1, 4066, -938, 1004),
    "shoulder_pitch_joint": (2, 3415, -362, 818),
    "elbow_pitch_joint": (3, 3995, -1024, 697),
    "head_roll_joint": (4, 827, -438, 877),
    "head_pitch_joint": (5, 4023, -1159, 1051),
}
EXPECTED_OFFSETS = {
    "base_yaw_joint": 2019,
    "shoulder_pitch_joint": 1368,
    "elbow_pitch_joint": 1948,
    "head_roll_joint": -1220,
    "head_pitch_joint": 1976,
}


def v2_document() -> dict[str, object]:
    joints = {
        name: {
            "servo_id": servo_id,
            "neutral_raw": neutral,
            "encoder_direction": 1,
            "safe_min_delta_raw": safe_min,
            "safe_max_delta_raw": safe_max,
            "lerobot_homing_offset": neutral - 2047,
        }
        for name, (servo_id, neutral, safe_min, safe_max) in V2_JOINTS.items()
    }
    joints["shoulder_pitch_joint"]["supported_rest_minimum_raw"] = 3053
    return {
        "schema_version": 1,
        "robot": "orion",
        "hardware": "v2",
        "servo_model": "sts3215",
        "encoder_resolution": 4096,
        "writes_servo_eeprom": False,
        "joints": joints,
    }


class FakeCentringBus:
    """Models the STS3215: present position = encoder - homing offset."""

    def __init__(self, applies_offset: bool = True, offsets: dict[str, int] | None = None) -> None:
        self.is_connected = False
        self.applies_offset = applies_offset
        self.offsets = offsets or {name: 0 for name in V2_JOINTS}
        self.encoder = {name: values[1] for name, values in V2_JOINTS.items()}
        self.writes: list[tuple[str, str, int]] = []

    def connect(self, handshake: bool = True) -> None:
        self.is_connected = True

    def read(self, data_name, motor, *, normalize=True, num_retry=0):
        assert data_name == "Homing_Offset"
        return self.offsets[motor]

    def write(self, data_name, motor, value, *, normalize=True, num_retry=0):
        self.writes.append((data_name, motor, value))
        if data_name == "Homing_Offset":
            self.offsets[motor] = value

    def sync_read(self, data_name, motors=None, *, normalize=True, num_retry=0):
        assert data_name == "Present_Position"
        return {
            name: (raw - (self.offsets[name] if self.applies_offset else 0)) % 4096
            for name, raw in self.encoder.items()
        }

    def disable_torque(self, motors=None, num_retry=0) -> None:
        pass

    def disconnect(self, disable_torque=True) -> None:
        self.is_connected = False


class CentringPlanTests(unittest.TestCase):
    def test_v2_offsets_put_every_zero_at_2047_and_clear_the_boundary(self) -> None:
        document = v2_document()
        steps = plan_centring(document)
        self.assertEqual({s.joint_name: s.target_offset_raw for s in steps}, EXPECTED_OFFSETS)
        self.assertEqual(
            {s.joint_name for s in steps if s.crosses_boundary},
            {"base_yaw_joint", "shoulder_pitch_joint", "elbow_pitch_joint", "head_pitch_joint"},
        )
        centred = centred_document(document, steps)
        for name, joint in centred["joints"].items():
            self.assertEqual(joint["neutral_raw"], 2047)
            self.assertEqual(joint["servo_homing_offset_raw"], EXPECTED_OFFSETS[name])
            self.assertEqual(joint["safe_min_delta_raw"], V2_JOINTS[name][2])
            self.assertFalse(crosses_encoder_boundary(
                joint["neutral_raw"], joint["safe_min_delta_raw"], joint["safe_max_delta_raw"]))
        # The supported shoulder rest keeps its delta of -362 from zero.
        self.assertEqual(centred["joints"]["shoulder_pitch_joint"]["supported_rest_minimum_raw"], 1685)
        self.assertIn("servo_homing_offsets_written_at", centred)

    def test_centred_calibration_needs_no_further_offset(self) -> None:
        document = v2_document()
        centred = centred_document(document, plan_centring(document))
        self.assertFalse(any(step.changes_offset for step in plan_centring(centred)))

    def test_rejects_documents_that_are_not_orion_calibrations(self) -> None:
        document = v2_document()
        document["encoder_resolution"] = 1024
        with self.assertRaises(CalibrationError):
            plan_centring(document)


class CentreCliTests(unittest.TestCase):
    def run_cli(self, bus: FakeCentringBus, answer: str = "CENTRE", extra=()):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        path = Path(directory.name) / "servo_calibration-v2.json"
        path.write_text(json.dumps(v2_document()), encoding="utf-8")
        stream = io.StringIO()
        with (
            patch("orion_servo_setup.centre_cli.create_lerobot_bus", return_value=bus),
            patch("orion_servo_setup.centre_cli.read_preflight"),
            patch("builtins.input", return_value=answer),
            redirect_stdout(stream),
        ):
            result = main(["--port", "/dev/fake", "--hardware", "v2", "--calibration", str(path), *extra])
        return result, path, stream.getvalue()

    def test_writes_verifies_relocks_and_saves_the_centred_calibration(self) -> None:
        bus = FakeCentringBus()
        result, path, output = self.run_cli(bus)
        self.assertEqual(result, 0, output)
        self.assertEqual(bus.offsets, EXPECTED_OFFSETS)
        self.assertEqual([w for w in bus.writes if w[0] == "Lock"], [("Lock", n, 1) for n in V2_JOINTS])
        saved = json.loads(path.read_text(encoding="utf-8"))
        self.assertEqual({j["neutral_raw"] for j in saved["joints"].values()}, {2047})
        self.assertEqual(len(list(path.parent.glob("*.backup-*"))), 1)

    def test_offsets_that_do_not_shift_positions_are_restored_and_file_kept(self) -> None:
        bus = FakeCentringBus(applies_offset=False)
        result, path, output = self.run_cli(bus)
        self.assertEqual(result, 1)
        self.assertIn("Restored the previous offsets", output)
        self.assertEqual(bus.offsets, {name: 0 for name in V2_JOINTS})
        self.assertEqual(json.loads(path.read_text(encoding="utf-8")), v2_document())

    def test_refuses_a_servo_whose_offset_does_not_match_the_calibration(self) -> None:
        offsets = {name: 0 for name in V2_JOINTS}
        offsets["elbow_pitch_joint"] = 500
        bus = FakeCentringBus(offsets=offsets)
        result, path, output = self.run_cli(bus)
        self.assertEqual(result, 1)
        self.assertIn("recalibrate instead", output)
        self.assertFalse([w for w in bus.writes if w[0] == "Homing_Offset"])

    def test_resumes_after_an_interrupted_run(self) -> None:
        offsets = {name: 0 for name in V2_JOINTS}
        offsets["base_yaw_joint"] = EXPECTED_OFFSETS["base_yaw_joint"]
        bus = FakeCentringBus(offsets=offsets)
        result, _, output = self.run_cli(bus)
        self.assertEqual(result, 0, output)
        self.assertEqual(bus.offsets, EXPECTED_OFFSETS)

    def test_cancel_and_dry_run_write_nothing(self) -> None:
        bus = FakeCentringBus()
        result, path, _ = self.run_cli(bus, answer="no")
        self.assertEqual(result, 2)
        self.assertEqual(bus.writes, [])
        self.assertEqual(json.loads(path.read_text(encoding="utf-8")), v2_document())
        result, _, output = self.run_cli(bus, extra=("--dry-run",))
        self.assertEqual(result, 0)
        self.assertFalse(bus.is_connected or bus.writes)
        self.assertIn("offset 0 -> 2019", output)


if __name__ == "__main__":
    unittest.main()
