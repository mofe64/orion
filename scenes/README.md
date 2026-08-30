# Orion scenes

This directory is the backend-independent source of truth for Orion's local
multimodal scenes. A scene references named poses and motions rather than
containing raw servo commands. Lighting is expressed as logical RGBW values;
the Pi backend is responsible for translating complete 40-pixel frames to the
physical NeoPixel shield.

Every file uses `format_version: 1` and an ordered timeline. Supported actions
in the first format are:

- `play_motion`: start a motion from `motion/motions/`.
- `goto_pose`: move to a pose from `motion/config/poses.yaml`.
- `light`: transition all pixels to an 8-bit RGBW value.
- `audio`: play a named local cue.

Example:

```yaml
format_version: 1

scene:
  name: acknowledge_left
  description: Turn left with a restrained warm acknowledgement light.
  timeline:
    - at: 0.0
      type: light
      red: 8
      green: 3
      blue: 0
      white: 20
      transition_seconds: 0.35
    - at: 0.0
      type: play_motion
      motion: look_at_left_expressive
```

`at` values use seconds from the scene's local monotonic start time. Events
must be ordered. A later motion event waits if the previous scene movement is
still executing or settling, while due light and audio events continue on the
timeline. A scene completes only after all events are dispatched, its final
light transition is complete, and its movement has completed and settled.

The `audio` action remains part of format version 1 for the next milestone, but
the physical backend is not implemented. If an authored scene contains an
audio event, its daemon run becomes `failed` with an explicit error; Orion does
not report silent placeholder playback as success.

`lighting_acknowledge` is the physical lighting-only commissioning scene. It
fades to `RGBW(8, 3, 0, 20)` and then returns to the warm-white idle value
`RGBW(0, 0, 0, 28)` without requiring torque or starting a movement.

With the source-run daemon active, submit and follow it with:

```bash
runtime/target/release/oriond --run-scene lighting_acknowledge --wait
runtime/target/release/oriond --scene-status
```

`acknowledge_left` additionally starts `look_at_left_expressive`, so the daemon
must be configured and holding before submission. Its scene run remains
`executing` while the underlying movement is executing or settling.
