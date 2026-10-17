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


## Preview all V2 movements

The preview script requires the simulator environment, Cargo and `ffmpeg`. It uses the Rust runtime compiler and scene scheduler at 50 Hz. V2 follows these samples with kinematic tracking.

1. Generate the movement videos and review index from the repository root.

   ```bash
   .venv/bin/python scripts/preview_movements.py
   ```

2. Open `../scratch/motion-previews/index.html` in a browser.

Each video shows side and front views, the item name, starting pose, return anchor and elapsed time. The camera encloses the lamp throughout the movement. Poses move from home to their target and back. A separate video shows startup from rest to home.

The [intent table](../../motion/config/v2/movement-intents.yaml) selects preview contexts from the runtime's ownership rules. Hands return starts at `desk_work` and targets the saved previous anchor. Arrival, pet and object-peek clips use their command-time planners. Other clips use their authored targets around the selected held anchor.

A disposable adapter compiles the unchanged runtime sources outside the repository. This gives the tool access to runtime planners without adding development commands to the app. The adapter uses the runtime Cargo lockfile.

The index lists non-adjacent contacts, clamps, joint-limit margins, peak joint speeds and final anchor errors. Each clip also shows head height change, gaze pitch change and its largest joint movement. Head height uses `v21_face`. Gaze pitch uses the forward axis of `light_frame`. Positive gaze change means looking up. Problems appear first.

The script excludes joined links and rest-to-rest no-op clips from problems. MuJoCo uses convex hulls for mesh contacts. The V2 base stays anchored. Kinematic tracking cannot check servo loads or base tipping.

`--output DIRECTORY` selects another directory outside the repository. `--compiler PATH` selects an existing `oriond` with catalog compilation support. `--validate-only` runs the checks without recording videos and returns an error for a problem. Videos use 25 frames per second by default. `--fps 50` records every sample.

`--baseline FILE` reads an earlier motion sample export for the comparison table.
