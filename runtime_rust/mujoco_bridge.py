#!/usr/bin/env python3
"""Line-delimited JSON bridge between the Rust runtime and native MuJoCo."""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path
from typing import Any

import mujoco


PROJECT_ROOT = Path(__file__).resolve().parent.parent
MUJOCO_DIRECTORY = PROJECT_ROOT / "simulation" / "mujoco"
sys.path.insert(0, str(MUJOCO_DIRECTORY))
sys.path.insert(0, str(PROJECT_ROOT / "motion"))

from mujoco_backend import (  # noqa: E402
    read_joint_positions,
    read_joint_velocities,
    resolve_joint_mapping,
    set_actuator_targets,
    set_joint_state,
)
from orion_motion.motion_loader import load_yaml_file  # noqa: E402
from stability_monitor import (  # noqa: E402
    StabilityMonitor,
    stability_policy_from_data,
)


JOINT_NAMES = (
    "base_yaw_joint",
    "shoulder_pitch_joint",
    "elbow_pitch_joint",
    "head_roll_joint",
    "head_pitch_joint",
)
RUNTIME_PERIOD = 0.02


def emit(value: dict[str, Any]) -> None:
    print(json.dumps(value, separators=(",", ":")), flush=True)


class Bridge:
    def __init__(self, scene: Path, start_positions: dict[str, float]) -> None:
        if set(start_positions) != set(JOINT_NAMES):
            raise ValueError("Start state must contain exactly Orion's five joints")
        self.model = mujoco.MjModel.from_xml_path(str(scene))
        self.data = mujoco.MjData(self.model)
        self.mapping = resolve_joint_mapping(self.model, JOINT_NAMES)
        set_joint_state(
            self.model,
            self.data,
            self.mapping,
            tuple(float(start_positions[name]) for name in JOINT_NAMES),
        )
        self.policy = stability_policy_from_data(
            load_yaml_file(PROJECT_ROOT / "motion/config/stability_limits.yaml")
        )
        self.monitor: StabilityMonitor | None = None
        self.latest_metrics = self._empty_metrics()

    def joint_limits(self) -> dict[str, list[float]]:
        values: dict[str, list[float]] = {}
        for name in JOINT_NAMES:
            joint_id = mujoco.mj_name2id(
                self.model, mujoco.mjtObj.mjOBJ_JOINT, name
            )
            values[name] = [
                float(self.model.jnt_range[joint_id][0]),
                float(self.model.jnt_range[joint_id][1]),
            ]
        return values

    def handle(self, request: dict[str, Any]) -> dict[str, Any]:
        command = request.get("command")
        if command == "activate":
            self.monitor = StabilityMonitor(self.model, self.data, self.policy)
            self.latest_metrics = self._empty_metrics()
            return self.state()
        if command == "deactivate":
            self.monitor = None
            return {"ok": True}
        if command == "write":
            positions = request.get("positions")
            if not isinstance(positions, dict) or set(positions) != set(JOINT_NAMES):
                raise ValueError("Position command must contain exactly Orion's joints")
            values = tuple(float(positions[name]) for name in JOINT_NAMES)
            if not all(math.isfinite(value) for value in values):
                raise ValueError("Position commands must be finite")
            set_actuator_targets(self.data, self.mapping, values)
            return {"ok": True}
        if command == "read":
            if request.get("advance"):
                steps = max(1, round(RUNTIME_PERIOD / self.model.opt.timestep))
                for _ in range(steps):
                    mujoco.mj_step(self.model, self.data)
                    if self.monitor is not None:
                        snapshot = self.monitor.update()
                        self.latest_metrics = {
                            "maximum_translation": snapshot.maximum_translation,
                            "maximum_tilt": snapshot.maximum_tilt,
                            "maximum_height_change": snapshot.maximum_height_change,
                            "longest_contact_loss": snapshot.longest_contact_loss,
                            "safe": snapshot.safe,
                            "unsafe_reasons": list(snapshot.unsafe_reasons),
                        }
            return self.state()
        raise ValueError(f"Unknown MuJoCo bridge command: {command}")

    def state(self) -> dict[str, Any]:
        positions = read_joint_positions(self.data, self.mapping)
        velocities = read_joint_velocities(self.data, self.mapping)
        return {
            "ok": True,
            "joints": [
                {
                    "name": name,
                    "position_rad": position,
                    "velocity_rad_s": velocity,
                    "current_ma": 0.0,
                    "voltage_v": 0.0,
                    "temperature_c": 0.0,
                    "status": 0,
                }
                for name, position, velocity in zip(
                    JOINT_NAMES, positions, velocities, strict=True
                )
            ],
            "metrics": self.latest_metrics,
        }

    @staticmethod
    def _empty_metrics() -> dict[str, Any]:
        return {
            "maximum_translation": 0.0,
            "maximum_tilt": 0.0,
            "maximum_height_change": 0.0,
            "longest_contact_loss": 0.0,
            "safe": True,
            "unsafe_reasons": [],
        }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--scene", type=Path, required=True)
    parser.add_argument("--start-json", required=True)
    args = parser.parse_args()
    try:
        bridge = Bridge(args.scene, json.loads(args.start_json))
        emit({"ok": True, "joint_limits": bridge.joint_limits()})
        for line in sys.stdin:
            try:
                request = json.loads(line)
                if request.get("command") == "shutdown":
                    return 0
                emit(bridge.handle(request))
            except Exception as error:  # keep protocol errors inspectable
                emit({"ok": False, "error": str(error)})
    except KeyboardInterrupt:
        return 0
    except Exception as error:
        emit({"ok": False, "error": str(error)})
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
