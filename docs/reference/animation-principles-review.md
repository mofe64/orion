# Orion v2 animation-principles review

This is the commissioning review for the built-in v2 character catalog. The
physical model does not deform: “squash and stretch” means coordinated joint
compression and extension. Every action is staged around one readable primary
idea; light, sound, and secondary joints support rather than compete with it.

## Motion review

| Asset | Primary action and silhouette | Anticipation / follow-through | Timing and secondary action |
|---|---|---|---|
| `look_at_left_expressive` | One broad leftward lamp-body arc ending in an attentive directional silhouette | Small opposite yaw prepares the turn; shoulder/head lag and an exact authored overshoot flow into settle | Expressive-turn timing; warm marker expression supports the readable turn |
| `look_at_right_expressive` | Mirrored rightward arc without mechanical symmetry in the supporting joints | Opposing preparation, layered lean, exact overshoot, weighted settle | Same dramatic beat as left while preserving calibrated right-side range |
| `attentive_entry` | Upward opening into a forward attentive silhouette | Compact preparation releases into head/shoulder lift; elbow follows | Quick attentive style; light focus is the secondary action |
| `acknowledge_nod` | Small clear nod around the held anchor | Head lead, restrained shoulder follow-through, clean return | Quick reaction with one phrase-scale beat, not repeated bobbing |
| `disagree_soft` | Restrained asymmetric side-to-side refusal | First side prepares the reversal; smaller counterbeat dissipates energy | Calm readable disagreement; muted light/cue remain subordinate |
| `curious_tilt` | Open diagonal head/body examination | Slight opposing base shift precedes the tilt; elbow settles last | Thinking-weighted timing and a spatial light sweep reinforce curiosity |
| `delight_lift` | Compact upward lift with an open, appealing silhouette | Small compression precedes extension; head and elbow finish after the body | Quick but restrained; sparkle and tonal cue land on the authored marker |
| `thinking_shift` | Asymmetric supported thinking pose | Lateral preparation and slow head follow-through avoid a generic lean | Slow thinking style; drifting warm light is the secondary action |
| `return_home` | Weighted downward/central settle into powered home | No decorative overshoot; joints arrive with controlled overlap | Slow return-home style communicates weight and finality |
| `look_at_left` | Functional direct left orientation | No expressive anticipation; continuous compiler still supplies slow-in/out | Utility motion keeps a clean directional silhouette |
| `look_at_right` | Functional direct right orientation | No expressive anticipation; final settle is explicit | Utility motion respects the same calibrated and spline contracts |
| `idle_breathe` | Coordinated visible rise and release | Compression/extension is restrained and always returns to the immutable anchor | Long living-idle timing; no sound |
| `idle_head_curiosity` | Gentle pitch/roll examination | First tilt flows through a smaller counter-shape | Head detail stays subordinate to the held pose; no sound |
| `idle_micro_glance` | Readable glance compatible with a held silhouette | Base starts the glance, head counters, both return | Short micro-idle with long randomized quiet interval |
| `idle_shoulder_adjust` | Small internal weight redistribution | Shoulder initiates; elbow/head trail and settle | Quiet living-idle timing; does not change the anchor |
| `idle_weight_shift` | Coordinated base/shoulder/elbow weight change | Body lead with head counterbalance and tapered return | Larger idle, selected less often; motion and light only |
| `idle_soft_head_shake` | Restrained asymmetric shake | Small first side, larger counter, diminished final echo | Direction changes flow through spline points without stop plateaus |
| `idle_attentive_hold` | Subtle upward energy within attentive anchors | Shoulder/head rise then diagonal secondary detail | Faster attentive character but low amplitude |
| `idle_directional_hold` | Detail that preserves a left/right held silhouette | Pitch/shoulder lead; roll/elbow follow | Avoids yaw that would undermine the directional staging |
| `speak_calm_sway` | Readable conversational head-and-body sway | Head, shoulder, and elbow form alternating counter-shapes and return cleanly | Used sparsely between quiet regions; smoothed light follows RMS energy |
| `speak_emphasis_nod` | Single phrase-boundary nod | Fast head preparation and smaller recoil | Emphatic style only at detected energy peaks |
| `speak_explanatory_lean` | Clear forward explanatory emphasis | Shoulder/elbow lead, head supports, then quiet return | Phrase-scale beat with deliberate stillness afterward |
| `speak_reflective_tilt` | Reflective diagonal thought shape | Base counterbalances the head tilt; diminished follow-through | Calm timing and longer quiet return; never loops continuously |

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

- Through keyframes preserve continuous position, velocity, and acceleration;
  a direction reversal may cross instantaneous zero velocity but never holds a
  zero-velocity plateau.
- Authored overshoot poses are exact; the compiler clamps extra overshoot.
- All relative idles and speech gestures use one uniform calibration-aware
  amplitude and end at zero offset from their immutable anchor.
- Routine idle has no sound, timers are randomized, and immediate repetition
  is excluded.
- Final `settle` is intentional and reaches zero velocity and acceleration.
