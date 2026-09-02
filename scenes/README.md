# Orion scene v2

Scenes coordinate named motion, RGBW effects, and local audio under one
monotonic clock. They never contain raw servo commands. All commissioned and
Studio-authored scenes use `format_version: 2`.

```yaml
format_version: 2
scene:
  name: acknowledge_left
  description: One fluid left turn with warm acknowledgement.
  motion:
    - {at: 0.0, play: look_at_left_expressive}
  lighting:
    - {at: 0.0, effect: attentive_focus, intensity: 0.65, duration: 1.6, transition: 0.2}
    - {on_marker: notice, effect: acknowledge_pulse, intensity: 0.8, duration: 0.65}
  audio:
    - {on_marker: notice, cue: acknowledge_warm}
  finish: {anchor: final_pose, lighting: pose_default}
```

The motion track is ordered and may not overlap. Lighting and audio may use an
absolute `at` time or a named motion `on_marker` trigger. Marker timing comes
from the retimed Rust trajectory, so expression remains synchronized when the
STS3215 speed ceiling stretches motion. One audio cue owns playback at a time;
due cues queue in authored order.

Completion waits for the final motion's measured settle, the last light effect
or transition, and audio playback. The final measured position becomes the
next character anchor, and the scene restores the nearest pose's
`default_lighting`. Cancellation stops scene-owned movement and audio.

The v2 effect vocabulary is `warm_idle_breathe`, `attentive_focus`,
`thinking_drift`, `speaking_energy`, `acknowledge_pulse`, `curious_sweep`,
`delight_spark`, `settle_glow`, and `off`. Every frame contains exactly 40
RGBW pixels for Orion's 8×5 matrix. Cue names resolve to WAV stems under
`audio/cues/`.

Built-ins live directly in `scenes/`; user scenes live in `scenes/user/`.
Studio publishes through the authenticated v2 gateway. The gateway writes
transactionally, asks `oriond` to validate and reload the complete catalog,
and rolls back a rejected update. Built-ins cannot be shadowed.

Run a scene through the owning daemon:

```bash
runtime/target/release/oriond --run-scene acknowledge_left --wait
runtime/target/release/oriond --scene-status
```

`deployment_smoke` is the lighting/audio-only v2 diagnostic. The physical
deployment pass also exercises both expressive look scenes. Mechanical `rest`
is not a scene: shutdown moves directly to the calibrated `rest` pose and only
then releases torque.
