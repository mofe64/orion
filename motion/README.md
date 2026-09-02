# Orion motion assets

This package owns Orion's built-in and user-authored pose and motion source
files. Runtime parsing and trajectory compilation live in Rust.

## Ownership

- `config/poses.yaml` contains built-in complete five-joint poses.
- `user/poses/` contains Studio-authored poses.
- `motions/expressive/` and `motions/functional/` contain absolute actions.
- `motions/idle/` and `motions/speaking/` contain anchor-relative character
  clips.
- `motions/user/` contains Studio-authored motions.
- `config/stability_limits.yaml` contains MuJoCo reporting policy; it is not a
  physical command limit.

The active Pi calibration is the hardware position authority. The tracked
`simulation/mujoco/config/servo_calibration.json` is its offline validation
counterpart. Rust is the only trajectory compiler; Python code validates and
consumes the exported 50 Hz sample document.

## Canonical documentation

- [Motion asset reference](../docs/reference/motion-assets.md) — pose and
  motion schemas, styles, catalog, and validation invariants.
- [Motion and animation architecture](../docs/explanation/motion-and-animation-architecture.md)
  — how intent becomes a physical action.
- [Character animation design](../docs/explanation/character-animation.md) —
  the 12 principles, idle behavior, and speech performance.
- [Trajectory and joint-control reference](../docs/reference/trajectory-and-joint-control.md)
  — compiler, runtime, calibration, and servo details.
- [Author and validate motion](../docs/how-to/author-and-validate-motion.md) —
  the required engineering workflow.

## Compile a portable trajectory

Generate a preview or diagnostic document with:

```bash
runtime/target/release/orion-trajectory \
  --motion look_at_left_expressive \
  --start-pose attentive \
  --pose-file motion/config/poses.yaml \
  --motions-directory motion/motions \
  --calibration simulation/mujoco/config/servo_calibration.json
```

For an anchor-relative motion, add `--anchor-pose POSE_NAME`. The exporter
loads the same assets and calibration used by the runtime, invokes the same
Rust compiler, and emits positions, velocities, accelerations, markers,
calibration ranges, and hardware-profile metadata.
