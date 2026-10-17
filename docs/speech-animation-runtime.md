# Speech animation runtime

The character coordinator turns speech analysis into continuous relative motion. The runtime owns playback and movement. Audio analysis does not send servo goals.

## Data flow

```text
PCM audio → speech analysis → character plan → trajectory compiler
                 │                                   ↓
                 └── playback position ───────→ movement updates
```

PCM means pulse-code modulation, a sequence of audio samples. Analysis supplies duration, energy, voiced regions, quiet intervals and local peaks. Reaction cues add named semantic gestures.

The main planner lives in `runtime/src/expression/character.rs`. Playback and waveform analysis live in `runtime/src/expression/speech.rs`.

## Coordinates and clocks

The anchor is the complete pose around which speech moves. Generated targets use offsets from that anchor. The selected hardware profile limits the resulting joint positions.

Runtime time measures execution. Audio time measures progress through the reply. Analysis uses 20 ms frames. Motion replacement also has local trajectory time. These clocks have different starting points.

Speech extensions preserve the original movement run ID. Each replacement increments its trajectory revision. Progress must use the matching run and current revision.

## Planning history

`SpeechMemory` stores the random generator, recent clips, direction choices, body state, emphasis time and planned cues. This history belongs to performed motion.

`plan_speech` saves that history before composing a candidate. Composition can change temporary planning state. The function restores performed history before returning the candidate and its checkpoints.

A checkpoint stores the history after one planned gesture. Planning does not count that gesture as performed. Rejected or replaced future plans therefore do not consume gesture history.

`advance_speech_memory` finds the active speech run and its current keyframe. It adopts only checkpoints strictly before that keyframe. It removes consumed checkpoints and applies the newest eligible snapshot once.

This progress follows the compiled timeline. It does not confirm physical arrival at every intermediate gesture. Final motion completion still uses measured settling.

## Compose a performance

The planner selects a bounded audio window. It keeps enough time for the final return. Long replies extend through later windows instead of planning the entire reply at once.

Quiet and energetic regions influence gesture choice. Head motion leads the action. Shoulder and elbow motion provide a later, smaller response. Emphasis gestures have a spacing limit.

Authored speaking clips supply motion shapes. The planner fits their offsets and timing to the audio window. One uniform amplitude factor keeps a relative shape inside the permitted range.

Yaw choice uses the model reference and anchor together. Thus, a coordinate-zero change does not change the intended facing direction. The planner alternates choices while avoiding recent repetition.

The compiler can stretch a segment to respect motor speed. The planner uses compiled timing when aligning gesture peaks. It limits alignment passes instead of repeatedly searching inside the movement loop.

## Reaction cues

Supported cues are `agree`, `disagree`, `happy`, `curious`, `thinking`, `surprised`, `sympathy`, `unsure` and `laugh`.

A reaction motion uses `anchor_relative`, returns to its anchor and has one apex marker. An apex marker identifies the gesture's main stroke. The runtime validates this contract when loading motions.

A reaction already underway can finish when its return fits the remaining allowance. A late cue cannot force unlimited motion after speech ends.

## Extend streaming speech

`tick_speaking` first updates performed history. It then checks whether new audio or an end marker requires another plan.

A successful replacement preserves the current commanded position, velocity and acceleration. It does not restart from delayed measured feedback. A failed replacement leaves the valid active plan in place.

The runtime verifies voice ownership and the return-to-anchor contract before accepting generated movement. The normal trajectory compiler and joint-limit checks still apply.

## Finish or interrupt

Speech end can preserve a valid reaction return. Otherwise, the coordinator creates a short return to the anchor. The character settles once after the reply.

Interruption cancels the old speech owner. A later session cannot continue its stale plan. Scene, Stop, rest and torque controls retain their normal priority.

Audio can continue when animation planning fails. The failure remains visible in runtime state. The planner never repairs a failure with direct servo writes.

## Change the planner

1. Locate the relevant planning function in `runtime/src/expression/character.rs`.
2. Preserve performed-history checkpoints when changing streaming behavior.
3. Preserve commanded-state continuity when changing replacement behavior.
4. Run the [runtime tests](../runtime/README.md#build-and-test).
5. Inspect the resulting motion with [MuJoCo](../simulation/mujoco/README.md).

Exact planner constants remain in the source file. Motion file fields are in the [motion reference](motion-reference.md).
