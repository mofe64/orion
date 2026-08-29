# Orion robot description

This directory contains simulator-independent robot-description assets.

- `urdf/orion.urdf` is the neutral kinematic, visual, collision, and inertial
  description. Mesh references are relative and require no ROS package index.
- `meshes/` is the single shared geometry source used by both the URDF and
  MuJoCo.

Simulator control plugins and launch configuration belong in their backend,
not in the neutral URDF. Orion's executable calibrated zero and joint ranges
remain encoded in `simulation/mujoco/robot.xml` and checked against the tracked
servo calibration.
