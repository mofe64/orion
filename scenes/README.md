# Orion scenes

Scenes combine motion, light and audio under one monotonic clock. A monotonic clock measures elapsed time without wall-clock adjustments. Scenes use `format_version: 2`.

## Schema

This scene starts a motion and uses its marker for light and audio:

```yaml
format_version: 2
scene:
  name: acknowledge_left
  description: Turn left with a warm acknowledgement.
  motion:
    - {at: 0.0, play: look_at_left_expressive}
  lighting:
    - {at: 0.0, effect: attentive_focus, intensity: 0.65, duration: 1.6, transition: 0.2}
    - {on_marker: notice, effect: acknowledge_pulse, intensity: 0.8, duration: 0.65}
  audio:
    - {on_marker: notice, cue: acknowledge_warm}
  finish: {anchor: final_pose, lighting: pose_default}
```

Motion clips are ordered and cannot overlap. Light and audio events use either `at` seconds or an `on_marker` trigger. They cannot use both fields. Markers follow the compiler's adjusted motion timing.

Completion waits for motion settling, light completion and audio completion. Cancellation stops scene-owned movement and audio. The final measured pose becomes the character anchor.

## Light and audio fields

Named light effects include `warm_idle_breathe`, `attentive_focus`, `thinking_drift`, `speaking_energy`, `acknowledge_pulse`, `curious_sweep`, `delight_spark`, `settle_glow` and `off`.

Custom effects use `constant`, `pulse`, `breathe` or `fade`. `colors` has one stage for constant output and two for other effects. Each stage uses `warm_white` or a `#rrggbb` value.

`levels` sets stage brightness from zero to one. `intensity` scales the complete effect. `period` controls pulse or breathe cycles and must be at least 0.1 seconds. Fade uses `duration`.

The hardware profile selects 40 RGBW pixels for V1 or 24 for V2. Audio cue names resolve to WAV stems in `audio/cues/`.

## Files and publication

Built-ins live in `scenes/v1/` and `scenes/v2/`. Each version's `user/` directory stores user scenes. User names cannot replace built-ins.

An optional top-level `studio` object carries scene-owned poses and motions. The gateway installs these dependencies before runtime reload. Copying the scene file alone does not install them.

## Run a scene

The owning daemon must be running. The following example uses its source-build client.

1. Run the scene and wait for completion.

   ```bash
   runtime/target/release/oriond --run-scene acknowledge_left --wait
   ```

2. Read scene status.

   ```bash
   runtime/target/release/oriond --scene-status
   ```

See the [Studio editor](../orion_studio/README.md#animation-library-and-scene-editor).
