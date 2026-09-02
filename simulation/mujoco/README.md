# Orion MuJoCo tools

MuJoCo implements Orion's simulation `RuntimeDriver`. It receives the same
Rust-compiled joint targets, 50 Hz lifecycle, markers, cancellation, and
measured-settling logic as the hardware runtime. It adds physics and diagnostic
reporting; it does not own a second interpolation algorithm.

Read the [motion and animation architecture](../../docs/explanation/motion-and-animation-architecture.md)
and [trajectory and joint-control reference](../../docs/reference/trajectory-and-joint-control.md)
before changing the backend or motion-player contract. Follow
[Author and validate Orion motion](../../docs/how-to/author-and-validate-motion.md)
for the complete engineering workflow.

## Calibrated pose editor

`pose_editor.py` is the visual editor for Orion's existing named poses. It is
separate from `pose_tuner.py`: the editor browses and saves the canonical pose
library, while the tuner remains a non-writing numeric development tool.

From the Orion repository, validate the model, calibration, and every pose
without opening windows:

```bash
../mujoco-local/.venv/bin/python simulation/mujoco/pose_editor.py \
  --check
```

Open the editor and MuJoCo viewer:

```bash
../mujoco-local/.venv/bin/python simulation/mujoco/pose_editor.py
```

The default editable library is
`motion/config/poses.yaml`. Use Previous/Next or the pose
selector to browse, move the five sliders for live preview, and choose Save
pose (or press Ctrl+S) to update the selected pose. Alt+Left and Alt+Right also
cycle through poses. Unsaved edits trigger a save/discard prompt when changing
poses.

The default calibration is the accepted 2026-08-29 snapshot in
`simulation/mujoco/config/servo_calibration.json`. Slider endpoints are computed
from each joint's `safe_min_delta_raw`,
`safe_max_delta_raw`, `encoder_direction`, and the 4096-count encoder
resolution in the selected physical calibration file. Exact-value entries are
checked against those same bounds. The editor validates all poses at startup
and writes only the five numeric lines belonging to the selected pose.

Pass `--calibration` only when deliberately testing a different capture. A new
accepted physical zero also requires regenerating the MJCF joint references;
changing the editor's calibration alone updates limits but not model geometry.

Servo encoder indices do not encode the absolute angle between a mounted horn
and its CAD mesh. `config/model_reference.json` records the visual alignment
that supplies this missing correspondence while the calibration snapshot
continues to define physical zero and safe travel.
