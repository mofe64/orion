# Orion joint structure

Orion has five powered joints connected in a single serial chain from base to head. Moving any joint rotates its child link and every link mounted further up the arm.

The V2 joint chain is:

```text
v21_base
  → base_yaw_joint → v21_base_yaw_link
  → shoulder_pitch_joint → v21_shoulder_pitch_link (lower arm)
  → elbow_pitch_joint → v21_elbow_pitch_link (upper arm)
  → head_pitch_joint → v21_wrist_pitch_link
  → head_roll_joint → v21_neck_swivel_link (head)
      ├── camera_optical_frame (fixed)
      └── light_frame (fixed)
```

## Joint functions

| V2 ID | Joint | What it moves |
| --- | --- | --- |
| 1 | `base_yaw_joint` | Turns everything above the base left and right |
| 2 | `shoulder_pitch_joint` | Tilts the lower arm forward and back |
| 3 | `elbow_pitch_joint` | Tilts the upper arm relative to the lower arm |
| 4 | `head_pitch_joint` | Tilts the head up and down at the wrist |
| 5 | `head_roll_joint` | Swivels the head left and right on the neck |

Each joint angle is measured relative to its parent link, with `0` radians defined as Orion's folded mechanical rest pose. Because every transform builds on the joints below it, the head's world direction depends on the full arm chain, not just the two head joints.

[orion-v2.urdf](../../description/urdf/orion-v2.urdf) defines the mechanical chain, while [v2-joints.json](../../hardware/profiles/v2-joints.json) defines the actual permitted hardware command limits—model geometry alone never grants permission to drive past hardware bounds.

## How V1 differs

V1 places neck swivel (`head_roll_joint`, ID 4) *before* head tilt (`head_pitch_joint`, ID 5) in [orion.urdf](../../description/urdf/orion.urdf), has no camera or light optical frames, and uses per-lamp calibration instead of V2's mechanical-rest zero. See [hardware versions](../hardware-versions.md#joint-names-and-limits) and [URDF concepts](orion_urdf_basics.md) for details.
