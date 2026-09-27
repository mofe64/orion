# Orion animation catalogue

Orion stages each animation around one clear action. Light, sound, and
supporting joint movement reinforce that action.

## Motion review

| Animation                        | Primary action and silhouette                                                  | Anticipation / follow-through                                                                                              | Timing and secondary action                                                                                                                                                  |
| -------------------------------- | ------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `look_at_left_expressive`        | One broad leftward lamp-body arc ending in an attentive directional silhouette | Small opposite yaw prepares the turn; shoulder/head lag and an exact authored overshoot flow into settle                   | Expressive-turn timing; warm marker expression supports the readable turn                                                                                                    |
| `look_at_right_expressive`       | Mirrored rightward arc without mechanical symmetry in the supporting joints    | Opposing preparation, layered lean, exact overshoot, weighted settle                                                       | Same dramatic beat as left while preserving calibrated right-side range                                                                                                      |
| `attentive_entry`                | Upward opening into a forward attentive silhouette                             | Compact preparation releases into head/shoulder lift; elbow follows                                                        | Quick attentive style; light focus is the secondary action                                                                                                                   |
| `acknowledge_nod`                | Small clear nod around the held anchor                                         | Head lead, restrained shoulder follow-through, clean return                                                                | Quick reaction with one phrase-scale beat, not repeated bobbing                                                                                                              |
| `disagree_soft`                  | Restrained asymmetric side-to-side refusal                                     | First side prepares the reversal; smaller counterbeat dissipates energy                                                    | Calm readable disagreement; muted light/cue remain subordinate                                                                                                               |
| `curious_tilt`                   | Open diagonal head/body examination                                            | Slight opposing base shift precedes the tilt; elbow settles last                                                           | Thinking-weighted timing and a spatial light sweep reinforce curiosity                                                                                                       |
| `delight_lift`                   | Compact upward lift with an open, appealing silhouette                         | Small compression precedes extension; head and elbow finish after the body                                                 | Quick but restrained; sparkle and tonal cue land on the authored marker                                                                                                      |
| `thinking_shift`                 | Asymmetric supported thinking pose                                             | Lateral preparation and slow head follow-through avoid a generic lean                                                      | Slow thinking style; drifting warm light is the secondary action                                                                                                             |
| `return_home`                    | Weighted downward/central settle into powered home                             | No decorative overshoot; joints arrive with controlled overlap                                                             | Slow return-home style communicates weight and finality                                                                                                                      |
| `look_at_left`                   | Functional direct left orientation                                             | No expressive anticipation; continuous compiler still supplies slow-in/out                                                 | Utility motion keeps a clean directional silhouette                                                                                                                          |
| `look_at_right`                  | Functional direct right orientation                                            | No expressive anticipation; final settle is explicit                                                                       | Utility motion respects the same calibrated and spline contracts                                                                                                             |
| `idle_breathe`                   | Coordinated visible rise and release                                           | 80 ms small head start, then unchanged arm drawing and anchor return                                             | Long living-idle timing; no sound                                                                                                                                            |
| `idle_head_curiosity`            | Gentle pitch/roll examination                                                  | First tilt flows through a smaller counter-shape                                                                           | Head detail stays subordinate to the held pose; no sound                                                                                                                     |
| `idle_micro_glance`              | Readable glance compatible with a held silhouette                              | Base starts the glance, head counters, both return                                                                         | Fixation settle and 0.5–1.0 s hold; long randomized interval                                                                                                                         |
| `idle_shoulder_adjust`           | Small internal weight redistribution                                           | Shoulder initiates; elbow/head trail and settle                                                                            | Quiet living-idle timing; does not change the anchor                                                                                                                         |
| `idle_weight_shift`              | Coordinated base/shoulder/elbow weight change                                  | Small head-only start, then original body drawing and tapered return                                                                      | Larger idle, selected less often; motion and light only                                                                                                                      |
| `idle_soft_head_shake`           | Restrained asymmetric shake                                                    | Small first side, larger counter, diminished final echo                                                                    | Direction changes flow through spline points without stop plateaus                                                                                                           |
| `idle_attentive_hold`            | Subtle upward energy within attentive anchors                                  | Small head-only start, unchanged shoulder/elbow rise, then diagonal detail                                                                          | Faster attentive character but low amplitude                                                                                                                                 |
| `idle_directional_hold`          | Detail that preserves a left/right held silhouette                             | Small pitch start precedes unchanged shoulder drawing; roll/elbow follow                                                                                     | Avoids yaw that would undermine the directional staging                                                                                                                      |
| `speak_calm_sway`                | Readable conversational head-and-body sway                                     | Supplies a calm dominant drawing to the speech performance                                                            | Weighted toward ordinary phrases; quiet intervals can hold its phrase pose                                                                                                                |
| `speak_emphasis_nod`             | Clear phrase-boundary nod                                                      | Fast head drawing redirects the continuing body path                                                                       | Compiled stroke targets 0.17 s before an eligible audio peak; authored lift then drop                                                                                                         |
| `speak_explanatory_lean`         | Clear forward explanatory emphasis                                             | Shoulder/head drawing carries momentum into the next phrase                                                                | Phrase-scale staging inside the continuous performance                                                                                                                       |
| `speak_reflective_tilt`          | Reflective diagonal thought shape                                              | Authored roll sign supplies asymmetry; emphasis includes its pitch counter-shape                                                                    | Calm timing with a phrase hold when the audio pauses                                                                                                                               |
| Generated `thinking_head`        | Readable diagonal thought around the conversational anchor                     | Opposing preparation, dominant head lead, delayed secondary body follow, asymmetric counter-tilt and diminished resolution | Existing thinking style; internal drawings flow through, final anchor return settles; amber/teal/lavender light breathes smoothly; repeated thinking requests preserve the active movement |
| Generated `speaking_performance` | One fluid, head-led action spanning the whole utterance                        | Fresh onset prepares; every phrase stages head before body; eligible emphasis uses its authored counter-stroke; final resolution lifts toward the listener                        | Seeded variation and local RMS head scaling avoid a fixed cycle; matching yaw/roll signs bias the arc; compiled emphasis precedes audio peaks; energy-gated body beats have a three-phrase minimum interval; quiet pauses hold the phrase pose; the final return settles at the anchor                    |

Per-play idle variation mirrors yaw/roll, varies head/yaw amplitude and travel
tempo, and preserves arm offsets. Directional variants point inward.
`idle_breathe` is micro only; stillness remains intentional between clips.

`attention_left` and `attention_right` use small base-led conversational arcs:
small opposing anticipation, a committed lean, restrained authored overshoot,
and one final settle. Shoulder compression and elbow/head overlap support the
facing idea. Their approximately 20-degree targets preserve a forward eyeline
without copying the full directional turns. The temporary anchor is held with
purposeful stillness until speech or return; no decorative cue is played.
See the [ELEGNT expression model](elegnt.md) and the
[acceptance invariants](#acceptance-invariants) below.

## Pose and scene review

`home`, `attentive`, `thinking`, `curious`, `delight`, `look_left`, and
`look_right` are powered character anchors with distinct readable silhouettes.
`home` holds the lamp head on a forward, slightly lowered cartoon eyeline rather
than at the lower edge of calibrated pitch travel.
`zero_reference` is calibration-only. `rest` is a mechanically supported
shutdown pose and is intentionally excluded from character animation.
Anticipation, lean, and overshoot poses are transition drawings, not idle
anchors; their purpose is to shape one continuous arc.

Each multimodal scene has one dominant motion: directional acknowledgement,
agreement, disagreement, curiosity, delight, thinking, attentive entry, or
return home. Marker-triggered light and sound land on the dominant beat and do
not introduce competing action. `deployment_smoke` is the sole diagnostic
exception: it intentionally has no motion and verifies the RGBW/audio devices.

## Acceptance invariants

- Character spline replacements preserve commanded position, velocity and
acceleration unless calibration protection attenuates the offending joint's
starting derivatives. Measured starts use zero acceleration.
- Through keyframes preserve continuous position, velocity, and acceleration;
a direction reversal may cross instantaneous zero velocity but never holds a
zero-velocity plateau.
- Authored overshoot poses remain exact. The compiler reduces bordering
velocity and acceleration when a polynomial overshoots between those poses.
- All relative idles and speech gestures use one uniform calibration-aware
amplitude and end at zero offset from their immutable anchor.
- Speech composes relative drawings into one movement run and extends its
trajectory as audio arrives. A quiet interval can include a `settle` and hold
at the phrase pose. Only the final return settles at the anchor.
- Every speech phrase contains a readable head drawing. Ordinary shoulder and
elbow motion remains secondary; full body beats are energy-gated, never
adjacent, and limited to at most roughly one in three phrase drawings.
- Routine idle has no sound, timers are randomized, and immediate repetition
is excluded.
- Final `settle` is intentional and reaches zero velocity and acceleration.
