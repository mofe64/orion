"""Interactively tune Orion poses with exact numeric MuJoCo targets.

This is a simulator-specific development tool.  It deliberately reads Orion's
shared pose data and validation rules but never writes to the pose library.
"""

from __future__ import annotations

import argparse
import math
import queue
import re
import sys
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Literal

import mujoco
import mujoco.viewer

from mujoco_backend import (
    read_joint_positions,
    resolve_joint_mapping,
    set_actuator_targets,
    set_joint_state,
)


PROJECT_ROOT = Path(__file__).resolve().parents[2]
MOTION_SOURCE = PROJECT_ROOT / "motion"
CONFIG_DIRECTORY = MOTION_SOURCE / "config"
DEFAULT_SCENE = Path(__file__).resolve().parent / "scene.xml"

# Add the backend-independent motion source so this MuJoCo tool can reuse its
# loader and validator without coupling the motion library to the simulator.
sys.path.insert(0, str(MOTION_SOURCE))

from orion_motion.motion_loader import load_yaml_file  # noqa: E402
from orion_motion.motion_validator import validate_pose_library  # noqa: E402


PoseCommandMode = Literal["move", "set"]


@dataclass(frozen=True)
class PoseConfiguration:
    joint_order: tuple[str, ...]
    limits: dict[str, tuple[float, float]]
    initial_pose_name: str
    initial_targets: dict[str, float]


@dataclass
class TunerState:
    commands: queue.Queue[tuple[PoseCommandMode, dict[str, float]]] = field(
        default_factory=queue.Queue
    )
    measured_lock: threading.Lock = field(default_factory=threading.Lock)
    measured_positions: dict[str, float] = field(default_factory=dict)
    stop_requested: threading.Event = field(default_factory=threading.Event)


def load_pose_configuration(pose_name: str) -> PoseConfiguration:
    """Load and validate the canonical pose library, limits, and start pose."""

    limits_data = load_yaml_file(CONFIG_DIRECTORY / "motion_limits.yaml")
    poses_data = load_yaml_file(CONFIG_DIRECTORY / "poses.yaml")
    validate_pose_library(poses_data, limits_data)

    poses = poses_data["poses"]
    if pose_name not in poses:
        available = ", ".join(sorted(poses))
        raise ValueError(
            f"Unknown starting pose '{pose_name}'. Available poses: {available}"
        )

    joint_order = tuple(limits_data["joint_order"])
    limits = {
        name: (
            float(
                limits_data["joints"][name]["operational_position"]["lower"]
            ),
            float(
                limits_data["joints"][name]["operational_position"]["upper"]
            ),
        )
        for name in joint_order
    }
    initial_targets = {
        name: float(poses[pose_name]["positions"][name]) for name in joint_order
    }

    return PoseConfiguration(
        joint_order=joint_order,
        limits=limits,
        initial_pose_name=pose_name,
        initial_targets=initial_targets,
    )


def format_pose_yaml(pose_name: str, targets: dict[str, float]) -> str:
    """Return a reviewable pose block suitable for copying into poses.yaml."""

    lines = [
        f"  {pose_name}:",
        "    description: Candidate pose from MuJoCo; review before saving.",
        "    positions:",
    ]
    for name, value in targets.items():
        lines.append(f"      {name}: {value:.6g}")
    return "\n".join(lines)


def parse_target_entries(
    entries: dict[str, str], limits: dict[str, tuple[float, float]]
) -> dict[str, float]:
    """Parse finite target values and enforce Orion's configured limits."""

    targets: dict[str, float] = {}
    for name, text in entries.items():
        try:
            value = float(text)
        except ValueError as error:
            raise ValueError(f"{name} must be a number") from error

        if not math.isfinite(value):
            raise ValueError(f"{name} must be finite")

        lower, upper = limits[name]
        if not lower <= value <= upper:
            raise ValueError(
                f"{name}={value} is outside [{lower}, {upper}] radians"
            )
        targets[name] = value

    return targets


def run_tk_controls(configuration: PoseConfiguration, state: TunerState) -> None:
    """Run the typed control window on its own UI thread."""

    import tkinter as tk
    from tkinter import ttk

    root = tk.Tk()
    root.title("Orion MuJoCo Pose Tuner")
    root.resizable(False, False)

    container = ttk.Frame(root, padding=12)
    container.grid(row=0, column=0, sticky="nsew")

    ttk.Label(container, text="Pose name").grid(row=0, column=0, sticky="w")
    pose_name = tk.StringVar(value=configuration.initial_pose_name)
    ttk.Entry(container, textvariable=pose_name, width=25).grid(
        row=0, column=1, columnspan=2, sticky="ew", padx=(8, 0)
    )

    ttk.Label(container, text="Joint").grid(row=1, column=0, sticky="w", pady=(10, 2))
    ttk.Label(container, text="Target (rad)").grid(
        row=1, column=1, sticky="w", pady=(10, 2)
    )
    ttk.Label(container, text="Measured (rad)").grid(
        row=1, column=2, sticky="w", pady=(10, 2)
    )
    ttk.Label(container, text="Allowed range").grid(
        row=1, column=3, sticky="w", pady=(10, 2)
    )

    target_variables: dict[str, tk.StringVar] = {}
    target_entries: list[ttk.Entry] = []
    measured_variables: dict[str, tk.StringVar] = {}

    for row, name in enumerate(configuration.joint_order, start=2):
        lower, upper = configuration.limits[name]
        target = tk.StringVar(value=f"{configuration.initial_targets[name]:.6g}")
        measured = tk.StringVar(value="—")
        target_variables[name] = target
        measured_variables[name] = measured

        ttk.Label(container, text=name).grid(row=row, column=0, sticky="w", pady=2)
        entry = ttk.Entry(container, textvariable=target, width=14)
        entry.grid(row=row, column=1, sticky="ew", padx=(8, 8), pady=2)
        target_entries.append(entry)
        ttk.Label(container, textvariable=measured, width=15).grid(
            row=row, column=2, sticky="w", pady=2
        )
        ttk.Label(container, text=f"[{lower:.3f}, {upper:.3f}]").grid(
            row=row, column=3, sticky="w", pady=2
        )

    status = tk.StringVar(value="Enter exact targets, then choose how to apply them.")

    def read_targets() -> dict[str, float]:
        return parse_target_entries(
            {name: variable.get() for name, variable in target_variables.items()},
            configuration.limits,
        )

    def submit(mode: PoseCommandMode) -> None:
        try:
            targets = read_targets()
        except ValueError as error:
            status.set(str(error))
            return

        state.commands.put((mode, targets))
        if mode == "move":
            status.set("Targets applied through the actuators.")
        else:
            status.set("Joint state set instantly and actuator targets updated.")

    for entry in target_entries:
        entry.bind("<Return>", lambda _event: submit("move"))

    def restore_initial() -> None:
        for name, value in configuration.initial_targets.items():
            target_variables[name].set(f"{value:.6g}")
        submit("move")

    def copy_yaml() -> None:
        name = pose_name.get().strip()
        if not re.fullmatch(r"[a-z][a-z0-9_]*", name):
            status.set("Pose name must use lowercase letters, numbers, and underscores.")
            return
        try:
            targets = read_targets()
        except ValueError as error:
            status.set(str(error))
            return

        yaml_text = format_pose_yaml(name, targets)
        root.clipboard_clear()
        root.clipboard_append(yaml_text)
        status.set("YAML pose block copied to the clipboard; review before saving.")

    button_row = 2 + len(configuration.joint_order)
    ttk.Button(container, text="Move to targets", command=lambda: submit("move")).grid(
        row=button_row, column=0, sticky="ew", pady=(12, 4)
    )
    ttk.Button(container, text="Set instantly", command=lambda: submit("set")).grid(
        row=button_row, column=1, sticky="ew", padx=4, pady=(12, 4)
    )
    ttk.Button(container, text="Restore start pose", command=restore_initial).grid(
        row=button_row, column=2, sticky="ew", padx=4, pady=(12, 4)
    )
    ttk.Button(container, text="Copy YAML", command=copy_yaml).grid(
        row=button_row, column=3, sticky="ew", pady=(12, 4)
    )

    ttk.Label(container, textvariable=status, wraplength=650).grid(
        row=button_row + 1, column=0, columnspan=4, sticky="w", pady=(6, 0)
    )

    def refresh_measured_positions() -> None:
        if state.stop_requested.is_set():
            root.destroy()
            return

        with state.measured_lock:
            measured = state.measured_positions.copy()
        for name, value in measured.items():
            measured_variables[name].set(f"{value:.6f}")
        root.after(100, refresh_measured_positions)

    def close() -> None:
        state.stop_requested.set()
        root.destroy()

    root.protocol("WM_DELETE_WINDOW", close)
    root.after(100, refresh_measured_positions)
    root.mainloop()


def run_mujoco_simulation(
    scene_path: Path, configuration: PoseConfiguration, state: TunerState
) -> None:
    """Run MuJoCo stepping and viewing until either window requests shutdown."""

    model = mujoco.MjModel.from_xml_path(str(scene_path))
    data = mujoco.MjData(model)
    mapping = resolve_joint_mapping(model, configuration.joint_order)
    initial_positions = tuple(
        configuration.initial_targets[name] for name in configuration.joint_order
    )
    set_joint_state(model, data, mapping, initial_positions)

    try:
        with mujoco.viewer.launch_passive(model, data) as viewer:
            viewer.cam.lookat[:] = [0.02, 0.0, 0.20]
            viewer.cam.distance = 0.75
            viewer.cam.azimuth = 90
            viewer.cam.elevation = -10

            while viewer.is_running() and not state.stop_requested.is_set():
                started = time.perf_counter()
                with viewer.lock():
                    while True:
                        try:
                            mode, targets = state.commands.get_nowait()
                        except queue.Empty:
                            break
                        positions = tuple(
                            targets[name] for name in configuration.joint_order
                        )
                        if mode == "set":
                            set_joint_state(model, data, mapping, positions)
                        else:
                            set_actuator_targets(data, mapping, positions)

                    mujoco.mj_step(model, data)
                    measured = dict(
                        zip(
                            configuration.joint_order,
                            read_joint_positions(data, mapping),
                            strict=True,
                        )
                    )

                with state.measured_lock:
                    state.measured_positions = measured

                viewer.sync()
                remaining = model.opt.timestep - (time.perf_counter() - started)
                if remaining > 0:
                    time.sleep(remaining)
    finally:
        state.stop_requested.set()


def run_interactive(
    scene_path: Path, configuration: PoseConfiguration
) -> None:
    """Run Tk on the main thread and MuJoCo on a simulation worker thread."""

    state = TunerState(measured_positions=configuration.initial_targets.copy())
    simulation_thread = threading.Thread(
        target=run_mujoco_simulation,
        args=(scene_path, configuration, state),
        name="orion-pose-tuner-simulation",
    )
    simulation_thread.start()

    try:
        run_tk_controls(configuration, state)
    finally:
        state.stop_requested.set()
        simulation_thread.join(timeout=2.0)


def check_configuration(scene_path: Path, configuration: PoseConfiguration) -> None:
    """Validate files and MuJoCo name mappings without opening GUI windows."""

    model = mujoco.MjModel.from_xml_path(str(scene_path))
    resolve_joint_mapping(model, configuration.joint_order)

    print(f"Scene: {scene_path}")
    print(f"Starting pose: {configuration.initial_pose_name}")
    for name in configuration.joint_order:
        lower, upper = configuration.limits[name]
        value = configuration.initial_targets[name]
        print(f"  {name}: {value:.6g}  limits=[{lower:.6g}, {upper:.6g}]")


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Tune Orion MuJoCo poses using exact numeric targets."
    )
    parser.add_argument(
        "--pose",
        default="attentive",
        help="Named pose used as the starting values (default: attentive).",
    )
    parser.add_argument(
        "--scene",
        type=Path,
        default=DEFAULT_SCENE,
        help="MuJoCo scene XML path.",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="Validate configuration and model mappings without opening GUIs.",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_arguments()
    scene_path = args.scene.resolve()
    configuration = load_pose_configuration(args.pose)

    if args.check:
        check_configuration(scene_path, configuration)
    else:
        run_interactive(scene_path, configuration)


if __name__ == "__main__":
    main()
