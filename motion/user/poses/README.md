# Orion user poses

Orion Studio stores user-authored keyframe poses here, one versioned YAML file
per named pose. Commissioned poses remain in `motion/config/poses.yaml` and are
never edited or shadowed.

User poses are immutable after creation. To adjust a keyframe, save a new pose
name and update the user motion or scene to reference it. `oriond` loads this
directory recursively and validates every position through the active hardware
calibration or simulator joint limits before swapping the asset library.
