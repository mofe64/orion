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

## V2 camera and light frames

The V2 URDF attaches two fixed frames to `v21_neck_swivel_link`. Their nominal
CAD origins are in metres, relative to that link:

| Frame | Origin (X, Y, Z) | Forward axis |
| --- | --- | --- |
| `camera_optical_frame` | `(0, -0.082, 0.0157)` | Local +Z |
| `light_frame` | `(0, -0.077, 0.020)` | Local +Z, along the light beam |

Both use ROS optical axes: **+X right, +Y down, +Z forward**. At zero joint
angles these point along base −X, −Z and −Y respectively. The base frame is
the URDF root link, `v21_base`: +Z is up, −Y is ahead, and −X is the lamp's
own right. Positive base yaw turns right; positive head pitch tilts down.
The fixed joints use roll/pitch/yaw `(-π/2, 0, π)`.

MuJoCo's `camera_optical_frame` camera has the same origin and view direction,
but its native axes are +X right, +Y up and −Z forward. Convert its rotation to
ROS optical axes by negating its Y and Z columns. The `light_frame` site
already uses the URDF optical axes. Existing bodies and servo joints retain
their origins, axes and ranges.

### Rust forward kinematics

`runtime/src/motion/kinematics.rs` loads the URDF's joint origins and axes with
`Kinematics::from_urdf(path)`. Load it once outside the control tick.
`fk(&JointPositions) -> Result<Frames>` returns `camera` and `light` poses in
the base frame. Each `FramePose` contains a position in metres and a row-major
rotation matrix whose columns are the local axes expressed in the base.
`rotate_vector` transforms a direction; `transform_point` also applies the
translation.

The loader requires a connected tree of fixed and revolute joints, and fixed
camera and light mounts. FK requires exactly the model's movable joint names
with finite angles in radians. It performs no I/O, allocates nothing on valid
input and moves nothing. It does not apply the URDF's CAD ranges as servo
calibration; motion still requires the runtime's trajectory and calibration
checks.

The nominal model puts the camera near `(-0.007, -0.146, 0.460)` m at the
repository's V2 `home`, looking about 5° up. These CAD mounts do not establish
lens intrinsics or the fitted camera-to-head correction. Measure those before
using real camera pixels to locate targets.

### Reproduce the kinematics fixtures

Use the existing [MuJoCo Python environment](../simulation/mujoco/README.md#python-environment):

```bash
.venv/bin/python tests/generate_camera_fk_fixtures.py --check
.venv/bin/python -m pytest tests/test_robot_description.py simulation/mujoco/test_v2_model.py
```

The generator uses MuJoCo 3.10.0 to compute positions and full orientations at
zero, home, left/right looks and 20 poses seeded with 2101 inside the model's
joint ranges. It converts world poses to the base frame and camera axes to ROS
optical axes. Rust's FK tests read the committed
`runtime/tests/fixtures/camera_fk_v2.json` without calling MuJoCo, requiring
agreement within 1 mm and 0.5°. Python checks the source hashes and reproduces
the fixture values. These comparisons validate geometry, not physical safety.

After an intentional model or source-pose change, regenerate and review the
fixture with `.venv/bin/python tests/generate_camera_fk_fixtures.py`.
