from __future__ import annotations

import io
import unittest
from contextlib import redirect_stdout

from orion_servo_setup.cli import main


class CliTests(unittest.TestCase):
    def test_dry_run_prints_plan_without_loading_hardware(self) -> None:
        stream = io.StringIO()
        with redirect_stdout(stream):
            result = main(["--port", "/dev/not-opened", "--dry-run"])

        output = stream.getvalue()
        self.assertEqual(result, 0)
        self.assertIn("ID 5: head_pitch_joint", output)
        self.assertIn("ID 1: base_yaw_joint", output)
        self.assertIn("no serial port was opened", output)


if __name__ == "__main__":
    unittest.main()
