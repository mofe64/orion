# Speech animation runtime

While Orion speaks, it shouldn't freeze like a speaker box or wave randomly. The character coordinator analyzes the reply's audio waveform in short rolling windows and stitches together authored gestures—aligning any tagged reaction strokes (`happy`, `surprised`, `agree`, etc.) with the right moments in the audio—while `oriond`'s 50 Hz runtime executes every resulting joint command safely.

```text
PCM audio → speech analysis → character plan → trajectory compiler
                 │                                   ↓
                 └── playback position ───────→ movement updates
```

Audio analysis (`runtime/src/expression/speech.rs`) measures duration, loudness, voiced stretches, quiet pauses and local energy peaks from 20 ms chunks of PCM audio. The main planner (`runtime/src/expression/character.rs`) turns those features plus any Codex reaction tags into anchor-relative motion plans.

## Coordinates and clocks

Every speaking gesture moves around Orion's current anchor pose using offsets clamped by the active hardware profile.

Planning and playback coordinate four different clocks: runtime execution time, audio playback progress, 20 ms analysis frames and local trajectory time. Because streaming replies extend an active motion in place, every replacement keeps the original movement `run_id` while bumping its trajectory `revision`, ensuring updates only apply to the matching active run.

## Planning history

When planning ahead for streaming audio, Orion needs to avoid repeating the same gestures without corrupting its history if a draft plan gets replaced or cancelled:

- `SpeechMemory` stores the random generator, recent clips, side choices, body state, emphasis timing and planned cues for motion that has *actually played*.
- `plan_speech` snapshots that memory before composing a window, restores it afterward, and emits checkpoints after each planned gesture.
- `advance_speech_memory` adopts checkpoints only once the active run passes each corresponding keyframe on the compiled timeline, ensuring discarded or replaced future plans never pollute gesture history.

Timeline checkpoints track planned progress rather than physical settling; final completion still waits for measured joint settling after the reply ends.

## Composing a performance

Rather than planning a whole long reply upfront, the planner works one bounded audio window ahead and always reserves enough time at the end to return smoothly to the anchor.

- Quiet stretches favor gentle sways, while energetic peaks invite nods and emphasis beats (with spacing limits so Orion doesn't bob endlessly).
- The head leads each gesture, with the shoulder and elbow following slightly later and smaller.
- Authored speaking clips supply the shapes, uniformly scaled so they always fit inside joint limits; yaw turns combine the model reference and current anchor so zeroing never flips Orion's intended direction.
- Because the trajectory compiler may stretch fast segments to obey servo speed limits, the planner aligns reaction peaks against compiled timing within a bounded number of passes rather than iterating inside the 50 Hz control loop.

## Reaction cues

Supported cues are `agree`, `disagree`, `happy`, `curious`, `thinking`, `surprised`, `sympathy`, `sorry`, `shy`, `unsure` and `laugh`.

Every reaction motion must use `anchor_relative` coordinates, return to its anchor, and contain exactly one `apex` marker marking its main visual stroke—`oriond` validates this contract on load. A reaction already playing near the end of speech may finish if its return fits the remaining allowance, preventing late cues from keeping Orion moving long after the voice stops.

## Extending streaming speech

On each tick during playback, `tick_speaking` advances performed checkpoints and checks whether newly arrived audio chunks (or the end of the reply) need a fresh plan extension. Valid replacements splice smoothly from the current **commanded** position, velocity and acceleration (never jumping back to lagged measured feedback), while failed plans simply leave the current valid plan running.

## Finishing or interrupting

When speech ends, Orion keeps any valid return already underway or appends a short settle back to the anchor. Interrupting speech (with `"Hey Orion"`, a scene, Stop or rest) cancels speech ownership immediately so stale extensions can never resume. Even if animation planning fails entirely, audio playback continues normally and never bypasses `oriond` with direct servo writes.

## Changing the planner

1. Locate the planning logic in `runtime/src/expression/character.rs`.
2. Preserve performed-history checkpoints whenever changing streaming windows.
3. Preserve commanded-state continuity whenever changing trajectory replacement.
4. Run the [runtime tests](../runtime/README.md#build-and-test).
5. Inspect the resulting motion visually in [MuJoCo](../simulation/mujoco/README.md).

Exact planner constants live in `character.rs`; YAML motion fields are documented in the [motion reference](motion-reference.md).
