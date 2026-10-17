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

## Documentation

- [How Orion moves](../docs/motion-architecture.md): how a request becomes servo movement.
- [Motion reference](../docs/motion-reference.md): schemas, styles, catalogue,
  trajectory compiler and servo control, including how to export a trajectory.

V2 joint angles are radians from mechanical rest. The Python limit reader and
Rust compiler both accept `hardware/profiles/v2-joints.json`; Studio reads that
same profile. Absolute poses use the rest frame. Anchor-relative motion offsets
are differences between angles and do not change when the zero frame changes.

Installed Python wheels locate resources in their enclosing release, or the
current checkout for a separate diagnostic venv. `ORION_PROJECT_ROOT` and
`ORION_TRAJECTORY_COMPILER` select explicit locations. `compile_trajectory(...,
hardware="v2")` selects the shared rest profile and V2 catalogue; the existing
V1 diagnostic default is retained.
