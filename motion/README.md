# Orion motion assets

The Rust runtime loads pose and motion YAML files. It compiles joint trajectories for hardware and simulation. Python code consumes the exported samples.

## Files and coordinates

| Path | Content |
| --- | --- |
| `config/v1/poses.yaml` | V1 built-in poses |
| `config/v2/poses.yaml` | V2 built-in poses |
| `user/poses/<version>/` | User poses |
| `motions/<version>/` | Built-in motions |
| `motions/<version>/user/` | User motions |
| `config/stability_limits.yaml` | Simulator reporting settings |

V2 angles are radians from mechanical rest. `hardware/profiles/v2-joints.json` supplies its shared limits. V1 uses its servo calibration. Stability reporting settings do not define hardware command limits.

Absolute poses contain joint angles. Anchor-relative motions contain differences from a selected pose. Changing a joint's zero does not change relative offsets.

The [motion reference](../docs/motion-reference.md) defines schemas, styles and compiler behavior. The [motion architecture](../docs/motion-architecture.md) explains request ownership.

## Python development

Run these steps from the repository root.

1. Install locked dependencies with managed Python.

   ```bash
   uv sync --project motion --locked --managed-python
   ```

2. Run the motion tests with pytest.

   ```bash
   uv run --project motion --locked --managed-python --with pytest python -m pytest motion/test
   ```

`ORION_PROJECT_ROOT` selects resource files. `ORION_TRAJECTORY_COMPILER` selects the Rust compiler executable. Installed wheels find resources in their enclosing release.

The Python `compile_trajectory` function retains a V1 default. Its `hardware="v2"` argument selects the V2 profile and catalogue.
