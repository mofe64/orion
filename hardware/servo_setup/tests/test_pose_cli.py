from __future__ import annotations

import io
import json
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

import yaml

from orion_servo_setup.archived.pose_cli import main
from test_pose_execution import calibration_document, pose_document


class PoseCliTests(unittest.TestCase):
    def test_dry_run_resolves_real_pose_without_opening_hardware(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            calibration = Path(directory) / "calibration.json"
            poses = Path(directory) / "poses.yaml"
            calibration.write_text(json.dumps(calibration_document()), encoding="utf-8")
            poses.write_text(yaml.safe_dump(pose_document()), encoding="utf-8")
            stream = io.StringIO()
            with (
                patch(
                    "orion_servo_setup.archived.pose_cli.create_lerobot_bus",
                    side_effect=AssertionError("hardware bus must not be created"),
                ),
                redirect_stdout(stream),
            ):
                result = main(
                    [
                        "home",
                        "--port",
                        "/dev/not-opened",
                        "--calibration",
                        str(calibration),
                        "--poses",
                        str(poses),
                        "--dry-run",
                    ]
                )

        self.assertEqual(result, 0)
        self.assertIn("base_yaw_joint", stream.getvalue())
        self.assertIn("target_raw", stream.getvalue())
        self.assertIn("no serial port was opened", stream.getvalue())


if __name__ == "__main__":
    unittest.main()
