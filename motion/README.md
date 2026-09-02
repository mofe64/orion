# Orion motion v2

Orion has one motion system and one position-limit authority:

- `config/poses.yaml` is the semantic pose v2 library.
- `motions/` contains absolute and anchor-relative motion v2 assets.
- `motion/user/poses/` and `motions/user/` hold create-only Studio assets.
- the live Pi calibration owns hardware position bounds; the tracked
  `simulation/mujoco/config/servo_calibration.json` is the offline copy.
- `config/stability_limits.yaml` is MuJoCo reporting policy, not a runtime gate.
- Rust compiles every trajectory; Python simulation only consumes its 50 Hz
  sample document.

The deleted provisional limit tables and Python trajectory generators are not
compatibility inputs.

## Pose v2

Every pose contains all five joints in radians plus character metadata:

```yaml
format_version: 2
units: radians
poses:
  attentive:
    tags: [powered, attentive, idle_anchor]
    idle_profile: attentive
    default_lighting: attentive_focus
    positions: {base_yaw_joint: -0.30, shoulder_pitch_joint: -0.10, elbow_pitch_joint: -0.28, head_roll_joint: -0.65, head_pitch_joint: -0.22}
```

`home` is powered rest. Mechanical `rest` is tagged shutdown-only and must not
be used for character animation.

## Motion v2

Absolute keyframes reference poses. Anchor-relative keyframes provide partial
offsets; omitted joints mean zero offset. `arrival: through` preserves
continuous position, velocity, and acceleration. Only `settle` may hold, and
every final keyframe settles. Relative idle/speaking clips must set
`return_to_anchor: true` and end at zero offsets.

The Rust compiler builds the complete piecewise quintic Hermite spline at
once, begins from measured position and velocity, clamps unintended overshoot,
and retimes only when a segment would exceed the 7.4 V STS3215 no-load ceiling
of 52 RPM (about 5.45 rad/s). Hardware and MuJoCo sample that same trajectory
at 50 Hz.

Generate the portable preview/diagnostic document with:

```bash
runtime/target/release/orion-trajectory \
  --motion look_at_left_expressive \
  --start-pose attentive \
  --pose-file motion/config/poses.yaml \
  --motions-directory motion/motions \
  --calibration simulation/mujoco/config/servo_calibration.json
```

The style library controls tempo, spline character, amplitude, overlap, and
settle intent; it never redefines hardware limits. The animation review for
the commissioned catalog is in
[`docs/reference/animation-principles-review.md`](../docs/reference/animation-principles-review.md).
