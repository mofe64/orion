# Orion v2 user scenes

Studio publishes user-authored `format_version: 2` scenes here. Built-ins in
the parent directory are immutable and cannot be shadowed.

- Motion, light, and audio use parallel tracks.
- Motion clips may not overlap; other tracks may run alongside them.
- Timed and marker-triggered events cannot be mixed on one event.
- Finish policy is `final_pose` plus `pose_default`.
- Updates require the exact content revision Studio loaded.
- `oriond` validates the complete catalog before accepting a reload.
