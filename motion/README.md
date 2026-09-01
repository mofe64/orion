# Orion motion

Orion's backend-independent motion assets live in this directory.

- `config/poses.yaml` contains named joint-space poses.
- `config/motion_limits.yaml` contains portable position and dynamic limits.
- `config/forbidden_regions.yaml` and `config/stability_limits.yaml` contain
  validation policies shared by simulator adapters.
- `motions/` contains functional and expressive keyframe sequences.
- `user/poses/` contains Studio-authored, create-only named poses.
- `motions/user/` contains Studio-authored, create-only named motions.
- `orion_motion/` contains the Python validation and trajectory-generation
  library used by MuJoCo and offline tests.

The physical Rust runtime reads the same pose and motion YAML files directly.
Neither the assets nor the Python validation library require ROS.

Studio-authored poses contain all five joints in radians and must stay within
the running driver's commissioned limits. A user motion is an ordered list of
named poses with positive transition durations and non-negative holds. The
runtime uses its existing quintic interpolation between those keyframes; user
assets do not introduce a second trajectory engine or a raw joint stream.

User assets are immutable by name. To revise one, save a new semantic name and
update any scene that references it. This preserves built-in assets and keeps
the exact pose/motion used by an existing scene reproducible.
