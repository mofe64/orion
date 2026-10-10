"""Generate scalar camera/light FK fixtures from the pinned MuJoCo model.

Run from the checkout with .venv/bin/python. No renderer, camera capture or
Rust FK is used: these are independent expected values for Rust's unit tests.
"""

import argparse
import hashlib
import json
from pathlib import Path
import random

import mujoco
import numpy as np
import yaml


ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "runtime/tests/fixtures/camera_fk_v2.json"
SEED = 2101
RANDOM_COUNT = 20
SOURCES = (
    "description/urdf/orion-v2.urdf",
    "simulation/mujoco/v2/robot.xml",
    "simulation/mujoco/v2/scene.xml",
    "motion/config/v2/poses.yaml",
)


def build_fixture():
    model = mujoco.MjModel.from_xml_path(str(ROOT / SOURCES[2]))
    data = mujoco.MjData(model)
    # Derive active names, limits and qpos slots from MuJoCo, not Rust or URDF.
    joints = sorted(
        mujoco.mj_id2name(model, mujoco.mjtObj.mjOBJ_JOINT, index)
        for index in range(model.njnt)
    )
    limits = {
        name: model.joint(name).range.tolist()
        for name in joints
    }
    poses = yaml.safe_load((ROOT / SOURCES[3]).read_text())["poses"]
    inputs = [("zero", dict.fromkeys(joints, 0.0))]
    inputs.extend((name, poses[name]["positions"]) for name in ("home", "look_left", "look_right"))
    rng = random.Random(SEED)
    inputs.extend(
        (f"random_{index:02d}", {name: rng.uniform(*limits[name]) for name in joints})
        for index in range(RANDOM_COUNT)
    )

    # MuJoCo camera: right/up/back; ROS optical: right/down/forward.
    camera_to_optical = np.diag([1.0, -1.0, -1.0])
    cases = []
    for name, positions in inputs:
        data.qpos[:] = 0.0
        for joint, angle in positions.items():
            data.qpos[model.joint(joint).qposadr[0]] = angle
        mujoco.mj_forward(model, data)
        base = data.body("v21_base")
        world_to_base = base.xmat.reshape(3, 3).T

        def pose(world_position, world_rotation):
            return {
                "position": np.round(world_to_base @ (world_position - base.xpos), 12).tolist(),
                "rotation": np.round(world_to_base @ world_rotation, 12).tolist(),
            }

        camera = data.camera("camera_optical_frame")
        light = data.site("light_frame")
        cases.append({
            "name": name,
            "positions": positions,
            "camera": pose(camera.xpos, camera.xmat.reshape(3, 3) @ camera_to_optical),
            "light": pose(light.xpos, light.xmat.reshape(3, 3)),
        })
    return {
        "format_version": 1,
        "mujoco_version": mujoco.__version__,
        "seed": SEED,
        "random_count": RANDOM_COUNT,
        "frame_convention": "base-relative metres; rotation maps optical +X right, +Y down, +Z forward into base",
        "source_sha256": {
            path: hashlib.sha256((ROOT / path).read_bytes()).hexdigest()
            for path in SOURCES
        },
        "model_joint_limits": limits,
        "cases": cases,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="fail if the committed fixture needs regeneration")
    args = parser.parse_args()
    document = build_fixture()
    text = json.dumps(document, indent=2, sort_keys=True, allow_nan=False) + "\n"
    if args.check:
        if OUTPUT.read_text() != text:
            parser.exit(1, "Camera FK fixture differs; regenerate with the pinned MuJoCo environment.\n")
        print(f"Camera FK fixture matches MuJoCo ({len(document['cases'])} poses).")
    else:
        OUTPUT.write_text(text)
        print(f"Wrote {OUTPUT.relative_to(ROOT)} ({len(document['cases'])} poses).")


if __name__ == "__main__":
    main()
