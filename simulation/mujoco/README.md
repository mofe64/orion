# Orion MuJoCo tools

## Calibrated pose editor

`pose_editor.py` is the visual editor for Orion's existing named poses. It is
separate from `pose_tuner.py`: the editor browses and saves the canonical pose
library, while the tuner remains a non-writing numeric development tool.

From the Orion repository, validate the model, calibration, and every pose
without opening windows:

```bash
../mujoco-local/.venv/bin/python simulation/mujoco/pose_editor.py \
  --check \
  --calibration ../orion-migration-backup/orion/servo_calibration.json
```

Open the editor and MuJoCo viewer:

```bash
../mujoco-local/.venv/bin/python simulation/mujoco/pose_editor.py \
  --calibration ../orion-migration-backup/orion/servo_calibration.json
```

The default editable library is
`ros2_ws/src/orion_motion/config/poses.yaml`. Use Previous/Next or the pose
selector to browse, move the five sliders for live preview, and choose Save
pose (or press Ctrl+S) to update the selected pose. Alt+Left and Alt+Right also
cycle through poses. Unsaved edits trigger a save/discard prompt when changing
poses.

Slider endpoints are computed from each joint's `safe_min_delta_raw`,
`safe_max_delta_raw`, `encoder_direction`, and the 4096-count encoder
resolution in the selected physical calibration file. Exact-value entries are
checked against those same bounds. The editor validates all poses at startup
and writes only the five numeric lines belonging to the selected pose.

On Orion itself, the default calibration path is
`~/.config/orion/servo_calibration.json`, so `--calibration` can be omitted if
the editor is launched there. On another computer, pass the current calibration
copied from Orion rather than an older or generic set of limits.
