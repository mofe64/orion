# Orion robot description

Orion's simulator-independent robot-description assets include:

- `urdf/orion.urdf` is v1's neutral kinematic, visual, collision, and inertial
  description. Mesh references are relative and require no ROS package index.
- `meshes/` is the single shared geometry source used by both the URDF and
  MuJoCo.
- `urdf/orion-v2.urdf` matches V2.1 joint transforms, visual meshes and estimated
  inertias. It references portable meshes under `simulation/mujoco/v2/meshes/`.
  Its effort/velocity limits are zero placeholders and it has no collision
  elements; it is not a commissioned controller or contact model. See
  [V2 simulation and mass estimates](../docs/hardware-versions.md#cad-urdf-and-mujoco).

Simulator control plugins and launch configuration belong in their backend,
not in the neutral URDF. Orion's executable calibrated zero and joint ranges
for v1 remain encoded in `simulation/mujoco/robot.xml` and checked against the tracked
servo calibration.
