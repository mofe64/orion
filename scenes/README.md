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
    - at: 0.12
      type: audio
      cue: acknowledge
```

`at` values use seconds from the scene's local monotonic start time. Events
must be ordered. A later motion event waits if the previous scene movement is
still executing or settling, while due light and audio events continue on the
timeline. A scene completes only after all events are dispatched, its final
light transition is complete, its movement has completed and settled, and its
last audio cue has exited successfully.

Audio cue names resolve to WAV filename stems under `audio/cues/`. The library
is validated when the daemon starts. One cue may play at a time; a later due
audio event waits until the current cue completes. Cancellation stops active
playback, and an `aplay` failure makes the scene `failed` rather than reporting
silent success.

`lighting_acknowledge` is the physical lighting-only commissioning scene. It
fades to `RGBW(8, 3, 0, 20)` and then returns to the warm-white idle value
`RGBW(0, 0, 0, 28)` without requiring torque or starting a movement.

With the source-run daemon active, submit and follow it with:

```bash
runtime/target/release/oriond --run-scene lighting_acknowledge --wait
runtime/target/release/oriond --scene-status
```

`acknowledge_left` and `acknowledge_right` coordinate their expressive motion,
warm acknowledgement light, and the local `acknowledge` cue. The daemon must
be configured and holding before submission. The scene run remains `executing`
while its movement is executing or settling or its cue is still playing.

`return_to_rest` moves Orion to the captured mechanical `rest` pose over three
seconds while fading every pixel to `RGBW(0, 0, 0, 0)`. Use this scene instead
of the standalone `--lights-off` command while the source-run daemon owns the
NeoPixel device:

```bash
runtime/target/release/oriond --run-scene return_to_rest --wait
runtime/target/release/oriond --disable
```

Only disable torque after the scene reports `completed`.
