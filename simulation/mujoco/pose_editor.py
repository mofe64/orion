"""Edit Orion's named pose library against physical calibration limits.

This is a dedicated authoring tool, separate from the numeric pose tuner. It
cycles through the canonical pose library, previews exact joint positions in
MuJoCo, and writes only the selected pose's position values back to YAML.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import queue
import re
import tempfile
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import yaml


PROJECT_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_SCENE = Path(__file__).resolve().parent / "scene.xml"
DEFAULT_POSE_LIBRARY = (
    PROJECT_ROOT / "motion" / "config" / "poses.yaml"
)
DEFAULT_CALIBRATION = (
    Path(__file__).resolve().parent / "config" / "servo_calibration.json"
)
CANONICAL_JOINTS = (
    "base_yaw_joint",
    "shoulder_pitch_joint",
    "elbow_pitch_joint",
    "head_roll_joint",
    "head_pitch_joint",
)


@dataclass(frozen=True)
class EditorConfiguration:
    calibration_path: Path
    pose_library_path: Path
    joint_order: tuple[str, ...]
    limits: dict[str, tuple[float, float]]
    pose_names: tuple[str, ...]
    descriptions: dict[str, str]
    poses: dict[str, dict[str, float]]


@dataclass
class EditorState:
    commands: queue.Queue[dict[str, float]] = field(default_factory=queue.Queue)
    stop_requested: threading.Event = field(default_factory=threading.Event)
    viewer_error: str | None = None


def _read_json_mapping(path: Path) -> dict[str, Any]:
    try:
        root = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError(f"Could not read calibration '{path}': {error}") from error
    if not isinstance(root, dict):
        raise ValueError("Calibration root must be a mapping.")
    return root


def load_calibrated_limits(path: Path) -> dict[str, tuple[float, float]]:
    """Convert Orion's safe encoder-delta limits to joint radians."""

    path = path.expanduser().resolve()
    root = _read_json_mapping(path)
    if root.get("schema_version") != 1:
        raise ValueError("Calibration must use schema_version 1.")
    if root.get("robot") != "orion" or root.get("servo_model") != "sts3215":
        raise ValueError("Calibration is not for Orion STS3215 hardware.")
    if root.get("encoder_resolution") != 4096:
        raise ValueError("Calibration must use the STS3215 4096-count encoder.")
    if root.get("writes_servo_eeprom") is not False:
        raise ValueError("Calibration must retain software-only EEPROM provenance.")

    joints = root.get("joints")
    if not isinstance(joints, dict) or set(joints) != set(CANONICAL_JOINTS):
        raise ValueError("Calibration must contain Orion's five canonical joints.")

    radians_per_count = 2.0 * math.pi / 4096.0
    limits: dict[str, tuple[float, float]] = {}
    servo_ids: set[int] = set()
    for expected_servo_id, name in enumerate(CANONICAL_JOINTS, start=1):
        joint = joints[name]
        if not isinstance(joint, dict):
            raise ValueError(f"Calibration '{name}' must be a mapping.")

        values: dict[str, int] = {}
        for field_name in (
            "servo_id",
            "neutral_raw",
            "encoder_direction",
            "safe_min_delta_raw",
            "safe_max_delta_raw",
        ):
            value = joint.get(field_name)
            if type(value) is not int:
                raise ValueError(
                    f"Calibration '{name}.{field_name}' must be an integer."
                )
            values[field_name] = value

        servo_id = values["servo_id"]
        neutral = values["neutral_raw"]
        direction = values["encoder_direction"]
        safe_min = values["safe_min_delta_raw"]
        safe_max = values["safe_max_delta_raw"]
        if servo_id != expected_servo_id or servo_id in servo_ids:
            raise ValueError(f"Calibration '{name}' has an unexpected servo ID.")
        servo_ids.add(servo_id)
        if not 0 <= neutral < 4096:
            raise ValueError(f"Calibration '{name}' neutral is outside 0..4095.")
        if direction not in (-1, 1):
            raise ValueError(f"Calibration '{name}' direction must be -1 or +1.")
        if not (-2048 < safe_min < 0 < safe_max < 2048):
            raise ValueError(
                f"Calibration '{name}' safe range must contain zero and stay "
                "inside one half-turn."
            )

        endpoints = (
            safe_min * radians_per_count / direction,
            safe_max * radians_per_count / direction,
        )
        limits[name] = (min(endpoints), max(endpoints))

    return limits


def _read_yaml_mapping(path: Path) -> dict[str, Any]:
    try:
        root = yaml.safe_load(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, yaml.YAMLError) as error:
        raise ValueError(f"Could not read pose library '{path}': {error}") from error
    if not isinstance(root, dict):
        raise ValueError("Pose library root must be a mapping.")
    return root


def load_editor_configuration(
    calibration_path: Path,
    pose_library_path: Path = DEFAULT_POSE_LIBRARY,
) -> EditorConfiguration:
    """Load and validate every pose against physical safe travel."""

    calibration_path = calibration_path.expanduser().resolve()
    pose_library_path = pose_library_path.expanduser().resolve()
    limits = load_calibrated_limits(calibration_path)
    root = _read_yaml_mapping(pose_library_path)
    if root.get("format_version") != 2:
        raise ValueError("Pose library must use format_version 2 (v2 required).")
    if root.get("units") != "radians":
        raise ValueError("Pose library units must be radians.")

    raw_poses = root.get("poses")
    if not isinstance(raw_poses, dict) or not raw_poses:
        raise ValueError("Pose library must contain at least one named pose.")

    poses: dict[str, dict[str, float]] = {}
    descriptions: dict[str, str] = {}
    for pose_name, raw_pose in raw_poses.items():
        if not isinstance(pose_name, str) or not re.fullmatch(
            r"[a-z][a-z0-9_]*", pose_name
        ):
            raise ValueError(f"Invalid pose name: {pose_name!r}.")
        if not isinstance(raw_pose, dict):
            raise ValueError(f"Pose '{pose_name}' must be a mapping.")
        description = raw_pose.get("description", "")
        if not isinstance(description, str):
            raise ValueError(f"Pose '{pose_name}' description must be text.")
        raw_positions = raw_pose.get("positions")
        if not isinstance(raw_positions, dict) or set(raw_positions) != set(
            CANONICAL_JOINTS
        ):
            raise ValueError(f"Pose '{pose_name}' must contain Orion's five joints.")

        positions: dict[str, float] = {}
        for joint_name in CANONICAL_JOINTS:
            raw_value = raw_positions[joint_name]
            if isinstance(raw_value, bool) or not isinstance(raw_value, (int, float)):
                raise ValueError(
                    f"Pose '{pose_name}' {joint_name} must be numeric."
                )
            value = float(raw_value)
            lower, upper = limits[joint_name]
            if not math.isfinite(value):
                raise ValueError(f"Pose '{pose_name}' {joint_name} must be finite.")
            if not lower <= value <= upper:
                raise ValueError(
                    f"Pose '{pose_name}' {joint_name}={value:.6g} is outside its "
                    f"calibrated range [{lower:.6g}, {upper:.6g}] radians."
                )
            positions[joint_name] = value
        poses[pose_name] = positions
        descriptions[pose_name] = description

    return EditorConfiguration(
        calibration_path=calibration_path,
        pose_library_path=pose_library_path,
        joint_order=CANONICAL_JOINTS,
        limits=limits,
        pose_names=tuple(poses),
        descriptions=descriptions,
        poses=poses,
    )


def validate_targets(
    targets: dict[str, float], limits: dict[str, tuple[float, float]]
) -> None:
    if set(targets) != set(CANONICAL_JOINTS):
        raise ValueError("A saved pose must contain Orion's five canonical joints.")
    for joint_name in CANONICAL_JOINTS:
        value = targets[joint_name]
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise ValueError(f"{joint_name} must be numeric.")
        lower, upper = limits[joint_name]
        if not math.isfinite(float(value)) or not lower <= float(value) <= upper:
            raise ValueError(
                f"{joint_name}={value} is outside calibrated range "
                f"[{lower:.6g}, {upper:.6g}] radians."
            )


def replace_pose_positions(
    yaml_text: str,
    pose_name: str,
    targets: dict[str, float],
) -> str:
    """Replace one pose's values without reformatting the surrounding YAML."""

    lines = yaml_text.splitlines(keepends=True)
    pose_pattern = re.compile(rf"^  {re.escape(pose_name)}:\s*(?:#.*)?(?:\r?\n)?$")
    pose_starts = [
        index for index, line in enumerate(lines) if pose_pattern.match(line)
    ]
    if len(pose_starts) != 1:
        raise ValueError(f"Expected one YAML block for pose '{pose_name}'.")
    start = pose_starts[0]
    end = len(lines)
    next_pose_pattern = re.compile(r"^  [a-z][a-z0-9_]*:\s*(?:#.*)?(?:\r?\n)?$")
    for index in range(start + 1, len(lines)):
        if next_pose_pattern.match(lines[index]):
            end = index
            break

    positions_indices = [
        index
        for index in range(start + 1, end)
        if re.match(r"^    positions:\s*(?:#.*)?(?:\r?\n)?$", lines[index])
    ]
    if len(positions_indices) != 1:
        raise ValueError(f"Pose '{pose_name}' must contain one positions block.")
    positions_start = positions_indices[0]

    for joint_name in CANONICAL_JOINTS:
        value_pattern = re.compile(
            rf"^(      {re.escape(joint_name)}:\s*)[^#\r\n]*(\s*#.*)?(\r?\n)?$"
        )
        matches = [
            index
            for index in range(positions_start + 1, end)
            if value_pattern.match(lines[index])
        ]
        if len(matches) != 1:
            raise ValueError(
                f"Pose '{pose_name}' must contain one value for {joint_name}."
            )
        index = matches[0]
        match = value_pattern.match(lines[index])
        assert match is not None
        comment = match.group(2) or ""
        newline = match.group(3) or ""
        lines[index] = f"{match.group(1)}{targets[joint_name]:.8f}{comment}{newline}"

    return "".join(lines)


def save_pose(
    configuration: EditorConfiguration,
    pose_name: str,
    targets: dict[str, float],
) -> None:
    """Atomically save one validated pose while retaining YAML formatting."""

    if pose_name not in configuration.poses:
        raise ValueError(f"Unknown pose '{pose_name}'.")
    validate_targets(targets, configuration.limits)
    path = configuration.pose_library_path
    try:
        original = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        raise ValueError(f"Could not read pose library '{path}': {error}") from error
    updated = replace_pose_positions(original, pose_name, targets)

    # Parse the result before replacing the source file. The full configuration
    # reload after saving performs the physical-range validation as well.
    try:
        yaml.safe_load(updated)
    except yaml.YAMLError as error:
        raise ValueError(f"Refusing to save invalid YAML: {error}") from error

    temporary_name: str | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w",
            encoding="utf-8",
            dir=path.parent,
            prefix=f".{path.name}.",
            suffix=".tmp",
            delete=False,
        ) as temporary:
            temporary_name = temporary.name
            temporary.write(updated)
            temporary.flush()
            os.fsync(temporary.fileno())
        os.chmod(temporary_name, path.stat().st_mode)
        os.replace(temporary_name, path)
    except OSError as error:
        if temporary_name is not None:
            try:
                Path(temporary_name).unlink(missing_ok=True)
            except OSError:
                pass
        raise ValueError(f"Could not save pose library '{path}': {error}") from error


def run_tk_editor(configuration: EditorConfiguration, state: EditorState) -> None:
    """Run the pose browser and calibrated slider controls."""

    import tkinter as tk
    from tkinter import messagebox, ttk

    root = tk.Tk()
    root.title("Orion MuJoCo Pose Editor")
    root.minsize(820, 430)

    container = ttk.Frame(root, padding=14)
    container.grid(row=0, column=0, sticky="nsew")
    root.columnconfigure(0, weight=1)
    root.rowconfigure(0, weight=1)
    container.columnconfigure(1, weight=1)

    current_index = 0
    current_targets = configuration.poses[configuration.pose_names[0]].copy()
    dirty = False
    loading_pose = False
    preview_after_id: str | None = None

    pose_variable = tk.StringVar(value=configuration.pose_names[0])
    count_variable = tk.StringVar()
    description_variable = tk.StringVar()
    status_variable = tk.StringVar(
        value="Ready. Slider ranges come from servo calibration."
    )
    value_variables: dict[str, tk.DoubleVar] = {}
    entry_variables: dict[str, tk.StringVar] = {}

    ttk.Button(container, text="← Previous", command=lambda: navigate(-1)).grid(
        row=0, column=0, sticky="w"
    )
    pose_selector = ttk.Combobox(
        container,
        textvariable=pose_variable,
        values=configuration.pose_names,
        state="readonly",
        width=31,
    )
    pose_selector.grid(row=0, column=1, sticky="ew", padx=10)
    ttk.Button(container, text="Next →", command=lambda: navigate(1)).grid(
        row=0, column=2, sticky="e"
    )
    ttk.Label(container, textvariable=count_variable).grid(
        row=0, column=3, sticky="e", padx=(12, 0)
    )
    ttk.Label(container, textvariable=description_variable, wraplength=760).grid(
        row=1, column=0, columnspan=4, sticky="w", pady=(8, 14)
    )

    ttk.Label(container, text="Joint").grid(row=2, column=0, sticky="w")
    ttk.Label(container, text="Calibrated slider (radians)").grid(
        row=2, column=1, sticky="w"
    )
    ttk.Label(container, text="Exact value").grid(row=2, column=2, sticky="w")
    ttk.Label(container, text="Safe range").grid(row=2, column=3, sticky="w")

    def schedule_preview() -> None:
        nonlocal preview_after_id
        if preview_after_id is not None:
            root.after_cancel(preview_after_id)
        preview_after_id = root.after(
            25, lambda: state.commands.put(current_targets.copy())
        )

    def slider_changed(joint_name: str, raw_value: str) -> None:
        nonlocal dirty
        value = float(raw_value)
        entry_variables[joint_name].set(f"{value:.6f}")
        if loading_pose:
            return
        current_targets[joint_name] = value
        dirty = True
        status_variable.set("Unsaved changes.")
        schedule_preview()

    for row, joint_name in enumerate(configuration.joint_order, start=3):
        lower, upper = configuration.limits[joint_name]
        value_variable = tk.DoubleVar(value=current_targets[joint_name])
        entry_variable = tk.StringVar(value=f"{current_targets[joint_name]:.6f}")
        value_variables[joint_name] = value_variable
        entry_variables[joint_name] = entry_variable

        ttk.Label(container, text=joint_name).grid(
            row=row, column=0, sticky="w", pady=4
        )
        slider = tk.Scale(
            container,
            from_=lower,
            to=upper,
            resolution=0.001,
            orient=tk.HORIZONTAL,
            showvalue=False,
            variable=value_variable,
            command=lambda value, name=joint_name: slider_changed(name, value),
        )
        slider.grid(row=row, column=1, sticky="ew", padx=(8, 12), pady=2)
        entry = ttk.Entry(container, textvariable=entry_variable, width=12)
        entry.grid(row=row, column=2, sticky="w", padx=(0, 12))
        ttk.Label(container, text=f"[{lower:.3f}, {upper:.3f}]").grid(
            row=row, column=3, sticky="w"
        )

        def commit_entry(_event: object, name: str = joint_name) -> None:
            nonlocal dirty
            try:
                value = float(entry_variables[name].get())
            except ValueError:
                status_variable.set(f"{name} must be a number.")
                entry_variables[name].set(f"{current_targets[name]:.6f}")
                return
            lower_bound, upper_bound = configuration.limits[name]
            if not math.isfinite(value) or not lower_bound <= value <= upper_bound:
                status_variable.set(
                    f"{name} must remain in [{lower_bound:.6f}, {upper_bound:.6f}]."
                )
                entry_variables[name].set(f"{current_targets[name]:.6f}")
                return
            if math.isclose(value, current_targets[name], abs_tol=1e-12):
                entry_variables[name].set(f"{current_targets[name]:.6f}")
                return
            current_targets[name] = value
            value_variables[name].set(value)
            entry_variables[name].set(f"{value:.6f}")
            dirty = True
            status_variable.set("Unsaved changes.")
            schedule_preview()

        entry.bind("<Return>", commit_entry)
        entry.bind("<FocusOut>", commit_entry)

    def write_current_pose() -> bool:
        nonlocal configuration, dirty
        pose_name = configuration.pose_names[current_index]
        try:
            save_pose(configuration, pose_name, current_targets.copy())
            configuration = load_editor_configuration(
                configuration.calibration_path, configuration.pose_library_path
            )
        except ValueError as error:
            messagebox.showerror("Could not save pose", str(error), parent=root)
            status_variable.set(str(error))
            return False
        dirty = False
        status_variable.set(
            f"Saved '{pose_name}' to {configuration.pose_library_path}."
        )
        return True

    def confirm_navigation() -> bool:
        if not dirty:
            return True
        decision = messagebox.askyesnocancel(
            "Unsaved pose",
            "Save the current pose before switching?",
            parent=root,
        )
        if decision is None:
            return False
        if decision:
            return write_current_pose()
        return True

    def load_pose(index: int) -> None:
        nonlocal current_index, current_targets, dirty, loading_pose
        current_index = index % len(configuration.pose_names)
        pose_name = configuration.pose_names[current_index]
        current_targets = configuration.poses[pose_name].copy()
        loading_pose = True
        try:
            pose_variable.set(pose_name)
            count_variable.set(f"{current_index + 1} / {len(configuration.pose_names)}")
            description_variable.set(configuration.descriptions[pose_name])
            for joint_name in configuration.joint_order:
                value = current_targets[joint_name]
                value_variables[joint_name].set(value)
                entry_variables[joint_name].set(f"{value:.6f}")
        finally:
            loading_pose = False
        dirty = False
        state.commands.put(current_targets.copy())
        status_variable.set(f"Viewing '{pose_name}'.")

    def navigate(offset: int) -> None:
        if confirm_navigation():
            load_pose(current_index + offset)
        else:
            pose_variable.set(configuration.pose_names[current_index])

    def select_pose(_event: object) -> None:
        selected = pose_variable.get()
        selected_index = configuration.pose_names.index(selected)
        if selected_index == current_index:
            return
        if confirm_navigation():
            load_pose(selected_index)
        else:
            pose_variable.set(configuration.pose_names[current_index])

    def reload_current() -> None:
        if dirty and not messagebox.askyesno(
            "Discard changes?", "Reload and discard the current edits?", parent=root
        ):
            return
        load_pose(current_index)

    button_row = 3 + len(configuration.joint_order)
    ttk.Button(container, text="Save pose", command=write_current_pose).grid(
        row=button_row, column=0, sticky="ew", pady=(14, 4)
    )
    ttk.Button(container, text="Reload pose", command=reload_current).grid(
        row=button_row, column=1, sticky="w", padx=(10, 0), pady=(14, 4)
    )
    ttk.Label(container, textvariable=status_variable, wraplength=760).grid(
        row=button_row + 1, column=0, columnspan=4, sticky="w", pady=(8, 0)
    )

    def close() -> None:
        if dirty and not messagebox.askyesno(
            "Discard changes?", "Close and discard the current edits?", parent=root
        ):
            return
        state.stop_requested.set()
        root.destroy()

    pose_selector.bind("<<ComboboxSelected>>", select_pose)
    root.bind("<Alt-Left>", lambda _event: navigate(-1))
    root.bind("<Alt-Right>", lambda _event: navigate(1))
    root.bind("<Control-s>", lambda _event: write_current_pose())
    root.protocol("WM_DELETE_WINDOW", close)

    def check_viewer() -> None:
        if state.stop_requested.is_set():
            if state.viewer_error is not None:
                messagebox.showerror(
                    "MuJoCo viewer stopped", state.viewer_error, parent=root
                )
            root.destroy()
            return
        root.after(100, check_viewer)

    load_pose(0)
    root.after(100, check_viewer)
    root.mainloop()


def run_mujoco_viewer(
    scene_path: Path,
    configuration: EditorConfiguration,
    state: EditorState,
) -> None:
    """Preview the latest edited pose in a passive MuJoCo viewer."""

    import mujoco
    import mujoco.viewer

    from mujoco_backend import resolve_joint_mapping, set_joint_state

    try:
        model = mujoco.MjModel.from_xml_path(str(scene_path))
        data = mujoco.MjData(model)
        mapping = resolve_joint_mapping(model, configuration.joint_order)
        first_pose = configuration.poses[configuration.pose_names[0]]
        positions = tuple(first_pose[name] for name in configuration.joint_order)
        set_joint_state(model, data, mapping, positions)

        with mujoco.viewer.launch_passive(model, data) as viewer:
            viewer.cam.lookat[:] = [0.02, 0.0, 0.20]
            viewer.cam.distance = 0.75
            viewer.cam.azimuth = 90
            viewer.cam.elevation = -10

            while viewer.is_running() and not state.stop_requested.is_set():
                started = time.perf_counter()
                with viewer.lock():
                    latest: dict[str, float] | None = None
                    while True:
                        try:
                            latest = state.commands.get_nowait()
                        except queue.Empty:
                            break
                    if latest is not None:
                        positions = tuple(
                            latest[name] for name in configuration.joint_order
                        )
                        set_joint_state(model, data, mapping, positions)
                    mujoco.mj_step(model, data)
                viewer.sync()
                remaining = model.opt.timestep - (time.perf_counter() - started)
                if remaining > 0:
                    time.sleep(remaining)
    except Exception as error:
        state.viewer_error = str(error)
    finally:
        state.stop_requested.set()


def run_interactive(scene_path: Path, configuration: EditorConfiguration) -> None:
    state = EditorState()
    viewer_thread = threading.Thread(
        target=run_mujoco_viewer,
        args=(scene_path, configuration, state),
        name="orion-pose-editor-viewer",
    )
    viewer_thread.start()
    try:
        run_tk_editor(configuration, state)
    finally:
        state.stop_requested.set()
        viewer_thread.join(timeout=2.0)


def check_configuration(scene_path: Path, configuration: EditorConfiguration) -> None:
    """Validate calibration, every pose, and MuJoCo names without opening GUIs."""

    import mujoco

    from mujoco_backend import resolve_joint_mapping

    model = mujoco.MjModel.from_xml_path(str(scene_path))
    resolve_joint_mapping(model, configuration.joint_order)
    print(f"Scene: {scene_path}")
    print(f"Calibration: {configuration.calibration_path}")
    print(f"Pose library: {configuration.pose_library_path}")
    print(f"Poses: {len(configuration.pose_names)}")
    for name in configuration.joint_order:
        lower, upper = configuration.limits[name]
        print(f"  {name}: calibrated=[{lower:.6f}, {upper:.6f}] rad")


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Browse and edit Orion poses in MuJoCo using calibrated sliders."
    )
    parser.add_argument(
        "--calibration",
        type=Path,
        default=DEFAULT_CALIBRATION,
        help=f"Orion servo calibration JSON (default: {DEFAULT_CALIBRATION}).",
    )
    parser.add_argument(
        "--poses",
        type=Path,
        default=DEFAULT_POSE_LIBRARY,
        help="Pose library YAML to edit.",
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
        help="Validate inputs and model mappings without opening GUI windows.",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_arguments()
    try:
        configuration = load_editor_configuration(args.calibration, args.poses)
        scene_path = args.scene.expanduser().resolve()
        if args.check:
            check_configuration(scene_path, configuration)
        else:
            run_interactive(scene_path, configuration)
    except (OSError, ValueError) as error:
        raise SystemExit(f"error: {error}") from error


if __name__ == "__main__":
    main()
