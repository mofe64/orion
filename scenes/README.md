# Orion scenes

Orion's backend-independent multimodal scenes live in this directory. A scene
references named poses and motions rather than
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

Studio's **Pulse** and **Breathe** controls are authoring templates rather than
new scene actions. Pulse expands into a short rise and return; breathe expands
into two slower rise/return cycles. Each generated keyframe is an ordinary
`light` event that can be retimed or recolored individually, and the last event
returns to the RGBW state sampled when the template was inserted.

Studio can also place a named scene as a composite timeline clip and split a
motion or composite scene into editable parts. This is an editor-only feature:
scene clips are recursively flattened before save or submission, motions become
ordinary `goto_pose` events, and authored holds become visible Delay clips.
When saved, each delay is a safe same-pose `goto_pose`, preserving the complete
configured motion duration without adding a new action type. Saved files
continue to contain only the four version-1 actions listed above.

**Preview on Orion** does not create a user file. Studio sends a small temporary
version-1 document through the authenticated gateway; `oriond` validates every
named pose, motion, and cue, starts the normal scene lifecycle, and discards the
definition after the run. Save/Publish remains the explicit path for adding a
scene to `scenes/user/`.

Audio cue names resolve to WAV filename stems under `audio/cues/`. The library
is validated when the daemon starts. One cue may play at a time; a later due
audio event waits until the current cue completes. Cancellation stops active
playback, and an `aplay` failure makes the scene `failed` rather than reporting
silent success.

`lighting_acknowledge` is the physical lighting-only commissioning scene. It
fades to `RGBW(8, 3, 0, 20)` and then returns to the warm-white idle value
`RGBW(0, 0, 0, 28)` without requiring torque or starting a movement.

`deployment_smoke` is a diagnostic-only scene used by `scripts/deploy_pi.sh`.
It exercises a restrained warm RGBW transition and the local `acknowledge` WAV
cue without commanding servo movement, then turns every pixel off. The
deployment script runs it only after the separate `zero_reference` movement
has completed.

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

Commissioned scenes remain directly in this directory. Studio-created copies
live under `scenes/user/`, and the runtime discovers both recursively. The
authenticated Studio gateway may list and read the Pi user library, create a
new user file, or revision-update an existing user file before requesting the
private Pi-local `scene reload` command. `oriond` rebuilds and validates the
entire catalog before swapping it in, refuses reload during an active scene,
and never changes the definition already owned by an executing run. A rejected
write is rolled back before the previous catalog is reloaded.
