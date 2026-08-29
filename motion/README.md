# Orion motion

This directory is the backend-independent source of truth for Orion motion.

- `config/poses.yaml` contains named joint-space poses.
- `config/motion_limits.yaml` contains portable position and dynamic limits.
- `config/forbidden_regions.yaml` and `config/stability_limits.yaml` contain
  validation policies shared by simulator adapters.
- `motions/` contains functional and expressive keyframe sequences.
- `orion_motion/` contains the Python validation and trajectory-generation
  library used by MuJoCo and offline tests.

The physical Rust runtime reads the same pose and motion YAML files directly.
Neither the assets nor the Python validation library require ROS.
