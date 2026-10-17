# Orion robot description

`description/urdf/` contains the Unified Robot Description Format models. URDF describes links, joints, joint origins and limits. `description/meshes/` contains their mesh assets.

## Models

| Version | URDF | MuJoCo scene |
| --- | --- | --- |
| V1 | `urdf/orion.urdf` | `../simulation/mujoco/scene.xml` |
| V2 | `urdf/orion-v2.urdf` | `../simulation/mujoco/v2/scene.xml` |

V2 uses mechanical-rest joint coordinates. Its URDF origins and MuJoCo references define the same zero geometry. `simulation/mujoco/config/model_reference-v2.json` stores model reference values.

Joint limits come from the selected hardware configuration. V2 uses `hardware/profiles/v2-joints.json`. Keep model limits consistent with that profile.

## Change a model

1. Change the relevant URDF joint or mesh reference.
2. Apply the corresponding change to the MuJoCo model.
3. Run the geometry tests from the repository root.

   ```bash
   .venv/bin/python -m pytest tests/test_robot_description.py tests/test_v2_rest_frame.py
   ```

The geometry tests require MuJoCo and pytest in the selected Python environment. See [MuJoCo setup](../simulation/mujoco/README.md).

The [V1 learning notes](../docs/learning_notes/README.md) explain links, frames and joint origins.
