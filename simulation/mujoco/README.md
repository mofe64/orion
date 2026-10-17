# Orion MuJoCo tools

MuJoCo provides the runtime's simulation driver. It receives the same compiled joint targets, markers, cancellation requests and completion checks as hardware.

V1 uses physical simulation. V2 uses a fixed-base CAD model with kinematic joint tracking. Kinematic tracking follows requested joint positions without simulating full lamp dynamics.

## Python environment

Run these steps from the repository root.

1. Create the simulator environment with managed Python 3.12.

   ```bash
   uv venv --managed-python --python 3.12 .venv
   ```

2. Install simulator and test dependencies.

   ```bash
   uv pip install --python .venv/bin/python -r simulation/mujoco/requirements.txt pytest
   ```

The Rust bridge uses `.venv/bin/python` by default.

## Run V2

1. Build the runtime.

   ```bash
   cargo build --manifest-path runtime/Cargo.toml --locked
   ```

2. Start the V2 simulator daemon.

   ```bash
   runtime/target/debug/oriond --serve --backend mujoco --hardware v2 --start-pose home
   ```

3. Use the [runtime controls](../../runtime/README.md#control-the-daemon) from another terminal.

The V2 model files live in `simulation/mujoco/v2/`. Its joint configuration uses `hardware/profiles/v2-joints.json`.

## Calibrated pose editor

The pose editor edits V1's named pose library. The separate pose tuner provides numeric adjustment without writing the library.

1. Check the V1 model and pose library without opening a window.

   ```bash
   .venv/bin/python simulation/mujoco/pose_editor.py --check
   ```

2. Open the pose editor and MuJoCo viewer.

   ```bash
   .venv/bin/python simulation/mujoco/pose_editor.py
   ```

3. Select a pose.
4. Adjust its joint sliders.
5. Select **Save pose**.

The default library is `motion/config/v1/poses.yaml`. The default calibration is `simulation/mujoco/config/servo_calibration.json`. Slider limits follow that calibration.

Changing calibration limits does not change model geometry. URDF origins and MJCF references define that geometry. See the [robot description](../../description/README.md).

## Model checks and renders

1. Run geometry checks in the prepared environment.

   ```bash
   .venv/bin/python -m pytest tests simulation/mujoco/test_v2_model.py
   ```

2. Regenerate the V2 rest reference images.

   ```bash
   .venv/bin/python scripts/render_v2_rest.py
   ```
