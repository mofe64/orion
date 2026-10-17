"""Independent MuJoCo geometry checks; run with the simulation pytest environment."""
from pathlib import Path


def test_reach_pose_apices_and_entry_paths_clear_the_simulated_base():
    import mujoco
    import numpy as np
    import yaml

    root = Path(__file__).resolve().parents[1]
    poses = yaml.safe_load((root / "motion/config/v2/poses.yaml").read_text())["poses"]
    model = mujoco.MjModel.from_xml_path(str(root / "simulation/mujoco/v2/scene.xml"))
    data = mujoco.MjData(model)
    home = poses["home"]["positions"]
    for name in ("lean_reach_low", "lean_reach_left", "lean_reach_right"):
        reach = poses[name]["positions"]
        for amount in np.linspace(0, 1, 101):
            for joint in home:
                data.qpos[model.joint(joint).qposadr[0]] = home[joint] + amount * (reach[joint] - home[joint])
            mujoco.mj_forward(model, data)
            assert data.ncon == 0, (name, amount, [(c.geom1, c.geom2, c.dist) for c in data.contact])
        light = data.site("light_frame")
        position = light.xpos - data.body("v21_base").xpos
        direction = light.xmat.reshape(3, 3)[:, 2]
        hit = position - position[2] / direction[2] * direction
        assert direction[2] < 0
        assert 0.25 < np.linalg.norm(hit[:2]) < 0.40
        assert position[2] > 0.23
        assert np.linalg.norm(position[:2]) > 0.19
