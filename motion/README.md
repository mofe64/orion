# Orion motion assets

Orion loads built-in and user-authored pose and motion files from this
directory. The Rust runtime parses those assets and compiles their trajectories.

## Ownership

- `config/v1/poses.yaml` and `config/v2/poses.yaml` contain each hardware version's built-in complete five-joint poses.
- `user/poses/v1/` and `user/poses/v2/` contain Studio-authored poses.
- `motions/v1/` and `motions/v2/` contain each version's absolute actions and anchor-relative character clips.
- `motions/v1/user/` and `motions/v2/user/` contain Studio-authored motions.
- `config/stability_limits.yaml` contains MuJoCo reporting policy; it is not a
  physical command limit.

The active Pi calibration is the hardware position authority. The tracked
`simulation/mujoco/config/servo_calibration.json` is its offline validation
counterpart. Rust is the only trajectory compiler; Python code validates and
consumes the exported 50 Hz sample document.

Pi deployment updates built-in pose and motion YAML from the selected Git commit,
including replacing local edits to built-ins. It preserves user-authored assets
and calibration and backs up replaced or retired built-ins for rollback. See
[Pi deployment](../docs/quickstart.md#deploy-to-the-pi) for the physical smoke test
and release-switch sequence.

## Canonical documentation

- [Motion asset reference](../docs/motion-assets.md) — pose and
  motion schemas, styles, catalog, and validation invariants.
- [Motion and animation architecture](../docs/motion-and-animation-architecture.md)
  — how intent becomes a physical action.
- [Character animation design](../docs/character-animation.md) —
  the 12 principles, idle behavior, and speech performance.
- [Trajectory and joint-control reference](../docs/trajectory-and-joint-control.md)
  — compiler, runtime, calibration, and servo details.

## Compile a portable trajectory

Generate a preview or diagnostic document with:

```bash
runtime/target/release/orion-trajectory \
  --motion look_at_left_expressive \
  --start-pose attentive \
  --pose-file motion/config/v1/poses.yaml \
  --motions-directory motion/motions/v1 \
  --calibration simulation/mujoco/config/servo_calibration.json
```

For an anchor-relative motion, add `--anchor-pose POSE_NAME`. The exporter
loads the same assets and calibration used by the runtime, invokes the same
Rust compiler, and emits positions, velocities, accelerations, markers,
calibration ranges, and hardware-profile metadata.
