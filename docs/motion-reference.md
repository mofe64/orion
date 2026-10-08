# Motion reference

Exact schemas, values and catalogue for Orion motion. For how the pieces fit together, start with [how Orion moves](motion-architecture.md).

## Asset files

```text
motion/
├── config/
│   ├── v1/poses.yaml               V1 built-in complete poses
│   ├── v2/poses.yaml               V2 built-in complete poses
│   └── stability_limits.yaml       MuJoCo reporting policy only
├── user/poses/
│   ├── v1/**/*.yaml                V1 Studio-authored poses
│   └── v2/**/*.yaml                V2 Studio-authored poses
└── motions/
    ├── v1/
    │   ├── expressive/*.yaml       character actions
    │   ├── functional/*.yaml       direct utility actions
    │   ├── idle/*.yaml             anchor-relative ambient clips
    │   ├── speaking/*.yaml         source drawings for generated speech
    │   └── user/**/*.yaml          V1 Studio-authored motions
    └── v2/
        ├── *.yaml                 V2 built-in motions
        └── user/**/*.yaml          V2 Studio-authored motions
scenes/
├── v1/                            V1 built-in scenes and user/ scenes
└── v2/                            V2 built-in scenes and user/ scenes
```

The selected hardware profile supplies the version's pose, motion and scene paths.
Loaders traverse only those directories, recursively in sorted path order.
Semantic names are unique within each version and asset type. A user asset cannot
shadow a built-in or another user asset in its version.

Pi deployment replaces built-in pose, motion and scene YAML with the selected
commit's copies, including local edits to built-ins, which are backed up for
rollback. User-authored assets under `user/` and calibration are never touched.
The active Pi calibration is the hardware position authority; the tracked
`simulation/mujoco/config/servo_calibration.json` is the offline (V1) copy used
for tests. `motion/config/stability_limits.yaml` is MuJoCo reporting policy, not
a command limit.

### Joint vocabulary

Every complete position map uses exactly these five names, in radians:

1. `base_yaw_joint`
2. `shoulder_pitch_joint`
3. `elbow_pitch_joint`
4. `head_roll_joint`
5. `head_pitch_joint`

The order is the canonical `ORION_JOINT_NAMES` order used by calibration,
drivers, state snapshots, Studio, and MuJoCo.

## Pose schema

```yaml
format_version: 2
units: radians

poses:
  attentive:
    description: Forward-facing pose with upward, curious energy.
    tags: [powered, attentive, idle_anchor]
    idle_profile: attentive
    default_lighting: attentive_focus
    positions:
      base_yaw_joint: -0.30
      shoulder_pitch_joint: -0.10
      elbow_pitch_joint: -0.28
      head_roll_joint: -0.65
      head_pitch_joint: -0.04
```

### Pose fields

| Field              | Required | Contract                                                 |
| ------------------ | -------- | -------------------------------------------------------- |
| `format_version`   | Yes      | Integer `2`                                              |
| `units`            | No       | If present, must be `radians`                            |
| `poses`            | Yes      | Non-empty mapping keyed by unique semantic names         |
| `description`      | No       | Human-readable intent and silhouette                     |
| `tags`             | No       | Semantic-name list used for lifecycle and catalog policy |
| `idle_profile`     | No       | Semantic profile used by character idle selection        |
| `default_lighting` | No       | Must name a built-in lighting effect                     |
| `positions`        | Yes      | Exactly one finite radian value for every Orion joint    |

A semantic name is non-empty and contains only ASCII letters, digits,
underscore, or hyphen.

### Pose roles

Tags communicate intended use:

- `powered` identifies poses safe to hold with torque enabled.
- `idle_anchor` identifies stable character silhouettes.
- `transition` identifies a drawing used inside an action rather than held as
ambient state.
- `authored_overshoot` documents intentional target overshoot.
- `shutdown_only` and `mechanical` reserve `rest` for supported torque release.
- `calibration_reference` reserves `zero_reference` for calibration checks.

Tags are descriptive except where character code explicitly checks
`shutdown_only` or `mechanical`. Authors must not infer unimplemented policy
from an unrecognized tag.

## Motion schema

One file contains one motion:

```yaml
format_version: 2
motion:
  name: look_at_left_expressive
  description: Notice, lean toward, and settle on the predefined left target.
  space: absolute
  style: expressive_turn
  keyframes:
    - pose: look_left_anticipation
      duration: 0.25
      arrival: through
    - pose: look_left_lean
      duration: 0.40
      arrival: through
      marker: notice
    - pose: look_left_overshoot
      duration: 0.30
      arrival: through
    - pose: look_left
      duration: 0.35
      arrival: settle
      marker: settled
```

### Motion fields

| Field              | Required      | Contract                                                           |
| ------------------ | ------------- | ------------------------------------------------------------------ |
| `format_version`   | Yes           | Integer `2`                                                        |
| `motion`           | Yes           | One motion mapping                                                 |
| `name`             | Yes           | Unique semantic name                                               |
| `description`      | No            | User-facing purpose and acting intent                              |
| `space`            | Yes           | `absolute` or `anchor_relative`                                    |
| `style`            | Yes           | One named style from the table below                               |
| `return_to_anchor` | Relative only | Must be `true` for relative motion; prohibited for absolute motion |
| `keyframes`        | Yes           | Non-empty ordered list                                             |

### Keyframe fields

| Field      | Required      | Contract                                                                  |
| ---------- | ------------- | ------------------------------------------------------------------------- |
| `pose`     | Absolute only | Existing named pose; `offsets` must be absent                             |
| `offsets`  | Relative only | Partial finite joint map; `pose` must be absent; omitted joints mean zero |
| `duration` | Yes           | Finite seconds greater than zero before style tempo is applied            |
| `arrival`  | Yes           | `through` or `settle`                                                     |
| `hold`     | No            | Finite, non-negative seconds; greater than zero only with `settle`        |
| `marker`   | No            | Unique semantic name reached at the compiled arrival time                 |

The final keyframe must use `settle`. The final relative keyframe must have no
non-zero offsets.

## Target resolution

### Absolute motion

An absolute keyframe resolves directly to the complete target of its named
pose. The runtime validates every target against active driver calibration.

Use absolute motion when the final world-relative silhouette matters, such as
a directional look or an explicit return home.

### Anchor-relative motion

A relative keyframe begins with a complete immutable anchor and adds only its
listed offsets:

```text
resolved = anchor + style.amplitude × runtime_scale × offsets
```

`runtime_scale` is the largest uniform value from zero through one that keeps
every offset in every keyframe within active calibration. Absolute motions use
a scale of one.

Use relative motion for ambient idle, speaking, and reusable detail that must
work around several powered poses. Relative motion must return to its anchor;
it cannot establish another anchor.

## Arrival semantics

### `through`

The compiler derives internal velocity and acceleration from the neighboring
segments and style. Position, velocity, and acceleration match on both sides
of the keyframe. A direction reversal may have zero instantaneous velocity,
but the compiler inserts no hold.

### `settle`

The compiler sets velocity and acceleration to zero at arrival. A hold may
follow. Every motion ends with a settle because runtime completion needs an
intentional final target.

Markers do not change trajectory shape. They attach semantic timing to the
retimed keyframe arrival so scene light and audio remain synchronized.

## Motion styles

The runtime defines styles as compiled constants. They control artistic timing
and amplitude; calibration and motor limits are applied separately.

| Style               | Tempo | Tangent tension | Joint lag | Amplitude | Overshoot scale | Settle character | Intended use                                        |
| ------------------- | ----- | --------------- | --------- | --------- | --------------- | ---------------- | --------------------------------------------------- |
| `living_idle`       | 0.82  | 0.38            | 0.18      | 0.90      | 0.00            | 0.85             | Unhurried low-amplitude ambient motion              |
| `attentive`         | 1.08  | 0.58            | 0.12      | 1.00      | 0.15            | 0.58             | Upward attentive entry and hold detail              |
| `expressive_turn`   | 1.00  | 0.72            | 0.22      | 1.00      | 1.00            | 0.62             | Anticipation, lean, authored overshoot, settle      |
| `speaking_calm`     | 0.72  | 0.42            | 0.16      | 0.95      | 0.00            | 0.82             | Restrained conversational source clips              |
| `speaking_emphatic` | 1.12  | 0.62            | 0.12      | 1.00      | 0.18            | 0.62             | Generated utterance performance and phrase emphasis |
| `thinking`          | 0.68  | 0.36            | 0.24      | 0.62      | 0.08            | 0.88             | Slow asymmetric thought                             |
| `quick_reaction`    | 1.34  | 0.70            | 0.08      | 0.92      | 0.24            | 0.48             | Short decisive acknowledgement                      |
| `return_home`       | 0.74  | 0.32            | 0.20      | 1.00      | 0.00            | 1.00             | Weighted final return                               |

Interpretation:

- A higher `tempo` shortens authored segment duration.
- `tangent_tension` scales internal derivative energy.
- `joint_lag` changes derivative character across the ordered joint chain; it
is not a separate scheduler delay.
- `amplitude` scales anchor-relative offsets.
- `overshoot_scale` affects internal acceleration character; it never creates
permission to leave the interval between authored segment endpoints.
- `settle_character` changes the timing weight of settle segments.

## Asset validation

Validation happens in this order:

1. Serde rejects unknown fields and malformed types.
2. Pose loading checks version, units, names, complete joints, finite values,
  metadata, and duplicate names.
3. Motion loading checks version, names, styles, space-specific fields,
  durations, holds, markers, final settle, and anchor return.
4. `RuntimeCore` validates all absolute pose and motion targets against the
  active driver limits at startup and transactional reload.
5. Relative targets are uniformly scaled and validated when instantiated
  around a concrete anchor.
6. The trajectory compiler validates full joint maps, derivative inputs,
  calibration containment, and motor-speed retiming.

No loader silently drops an invalid field, clips an authored absolute target,
or substitutes a missing pose.

## Animation catalogue

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
See the [design qualities](motion-architecture.md#designing-animation) and the
[acceptance invariants](#acceptance-invariants) below.

### Built-in motions by group

**Expressive:** `look_at_left_expressive`, `look_at_right_expressive`, `attention_left`, `attention_right`, `attentive_entry`, `acknowledge_nod`, `disagree_soft`, `curious_tilt`, `delight_lift`, `thinking_shift`

**Functional:** `look_at_left`, `look_at_right`, `return_home`

**Idle:** `idle_breathe`, `idle_head_curiosity`, `idle_micro_glance`, `idle_shoulder_adjust`, `idle_weight_shift`, `idle_soft_head_shake`, `idle_attentive_hold`, `idle_directional_hold`

**Speaking source drawings:** `speak_calm_sway`, `speak_emphasis_nod`, `speak_explanatory_lean`, `speak_reflective_tilt`

The runtime-generated `speaking_performance` and interruption-only `speak_settle` are not YAML assets. `CharacterCoordinator` builds them in memory from waveform analysis and the speaking source drawings.

### Poses and scenes

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

### Acceptance invariants

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

## Idle behaviour details

The coordinator owns two independent monotonic deadlines:

- a micro-idle after a seeded random delay from 8 to 20 seconds;
- a larger idle after a seeded random delay from 35 to 75 seconds.

Whichever deadline is earlier is the next category. Completing an idle
reschedules only that category, preserving the other deadline. Speech, a
foreground action, or a reaction-state change resets the schedule so Orion
does not immediately add ambient movement after a user-facing action.

The scheduler seeds its pseudorandom generator. Physical runs still vary,
while tests and previews can reproduce the exact selection sequence. The
scheduler removes the immediately preceding clip from the candidate set.

The nearest pose's `idle_profile` adapts ambient movement to the held
silhouette:

- ordinary powered anchors use breathing, head curiosity, micro glance,
shoulder adjustment, weight shift, and soft head shake;
- attentive anchors may add `idle_attentive_hold`;
- directional anchors use `idle_directional_hold` and avoid yaw clips that
would collapse against the left or right calibration boundary.

Each play clones its selected asset, mirrors yaw and roll as a
pair with 50% probability, varies head/yaw amplitude by 0.85–1.15, and divides
travel durations by a seeded tempo factor of 0.9–1.1. Arm offsets retain their
authored magnitudes. Directional variants point yaw inward; mirrors
cannot push a lateral joint toward its nearby calibration limit.
`idle_breathe` belongs only to micro selection. `idle_micro_glance` reaches a
real settle at its look target, holds for 0.5–1.0 seconds per play, then returns.
There is no continuous idle layer or coupling to voice attention.

`idle_breathe`, `idle_attentive_hold`, `idle_directional_hold` and
`idle_weight_shift` start with an 80 ms head-only drawing at 12% of their
original first head target. Arms then follow to the unchanged authored
drawing. Total authored travel time and all original arm offsets stay intact;
tempo variation keeps the added head start inside 50–100 ms. This small
intermediate drawing avoids compressing the entire head stroke into 80 ms.

The two categories create contrast. Micro-idles are short details; larger
idles redistribute more of the body and happen less often.

Before compiling a relative clip, the runtime computes the largest single
scale in `[0, 1]` that keeps every styled offset inside the live calibrated
range around the anchor. The runtime applies one scale to the whole clip. This
retains the authored relationship among joints instead of flattening whichever
joint reaches its limit first.

The motion starts from measured position and velocity but resolves every
target from the immutable anchor. If foreground work arrives, `stop` cancels
the idle lifecycle and the foreground trajectory blends from the measured
interruption state. There is no forced trip back to the anchor before the
action.

The lowest-priority background light comes from the nearest anchor pose's
`default_lighting`; `warm_idle_breathe` is the fallback. Listening, thinking,
starting, and settling states select their corresponding restrained effects.

Routine idles do not play sound. Repetitive ambient audio makes autonomous
behavior feel like notification noise, while motion and low-intensity light
are sufficient to communicate life.

## Speech performance policy

Speech uses one continuous performance lifecycle. Each plan covers at most
20 seconds of audio, or all the remaining audio when no more than 25 seconds
remain. Longer complete files are planned in pieces and extended like streamed
audio, which keeps planning inside one 50 Hz cycle however long the reply is.
Streaming replies and later pieces extend the spline from commanded position,
velocity and acceleration after the current body follow and any hold finish. Newly received audio
does not replace a gesture midway through its head lead. Gesture history, random
state, and body shape advance only when the runtime passes a planned body-follow
checkpoint; composing an unperformed future does not advance that history. The motion run and immutable
anchor persist across extensions. Network chunks do not trigger separate gestures
or intermediate returns to rest. The stream end marker revises the remaining
plan at the next gesture boundary even when audio duration has not changed.
With at most 0.9 seconds remaining,
finalization installs only a settle rather than another expressive gesture.

### Audio analysis

The onboard coordinator sends the selected TTS model's audio as ordered RIFF/WAV chunks to the local gateway;
the complete-file endpoint also remains available. The gateway requires mono, 24 kHz, signed 16-bit pulse-code modulation
(PCM16). It applies size and duration limits, writes an atomic random spool
item, and asks the speech coordinator to start or append that identifier. The coordinator
never accepts an arbitrary path.

The runtime divides PCM into 20 ms frames, matching its 50 Hz loop. For each
frame it calculates root-mean-square (RMS) energy and applies exponential
smoothing:

```text
smoothed[n] = 0.65 × smoothed[n-1] + 0.35 × rms[n]
```

It derives:

- quiet regions below `max(12% of maximum energy, 0.004)`; the analyzer
discards internal runs shorter than three frames but retains a trailing quiet
run; and
- phrase peaks above `1.35 × mean energy`, locally maximal, and separated by
at least ten frames (200 ms).

The same analysis drives movement planning and the `speaking_energy` light.
The light has a faster attack than release, is capped below full brightness,
and remains secondary to physical acting.

### Planning the utterance

The character coordinator allocates the waveform duration between active
phrase motion and one final settle. It then plans phrase drawings with seeded
variation:

- ordinary phrases prefer `speak_calm_sway` and `speak_reflective_tilt`, with
occasional explanatory shapes;
- eligible peaks prefer `speak_emphasis_nod`, reflective tilts, or restrained sways;
  strong, sufficiently spaced peaks may receive an explanatory body beat;
- immediate clip repetition is excluded, and recent shapes receive lower
  selection weights without forcing a fixed three-clip cycle;
- duration varies around the phrase category's nominal timing;
- head roll may retain its direction, ease toward neutral, or change sides;
  yaw turns bias non-reflective roll toward matching signs (the same side in
  grounded-base MuJoCo), while reflective tilts keep their authored sign;
- head pitch supplies nods, lifts, and authored emphasis counter-strokes;
  mean RMS over each drawing's audio window gently scales its head amplitude;
- small base-yaw turns are chosen without repeating a direction and are
constrained away from a directional anchor's nearby limit.

The authored clips contribute approved character shapes. The generated
performance varies and layers those shapes rather than inventing unconstrained
joint targets.

### Head-led staging

Every planned phrase is divided into a head lead and body follow:

1. **Head lead:** roll, pitch, and optional yaw establish the phrase direction
  while the body retains the preceding secondary shape.
2. **Body follow:** shoulder and elbow arrive later while the head blends
  toward the next phrase's arc.

Ordinary head leads receive roughly two-thirds of the phrase duration. Emphasis
strokes receive 25–35%, followed by the authored second head drawing: the nod
lifts and then drops. Reflective tilts retain their authored roll sign.
Emphasis is planned backward from an audio peak, targeting a commanded apex
0.17 seconds before it; a preceding drawing shorter than 0.35 seconds is
not compressed to make room. Compiled arrival checks include style tempo and
speed retiming. All emphases in a plan are aligned together: one compile
measures every apex, corrections carry their time shift forward to later
emphases, and a second pass corrects retiming effects, so a plan compiles at
most three times. Peaks that cannot fit safely are skipped. During an ordinary
body follow, the head target includes an 18% look-ahead toward the following
phrase's head target. That staging creates anticipation and overlap inside the
same continuous spline. At selected quiet intervals, the body follow instead
settles at the phrase pose and holds briefly without returning to the anchor.

The planner scales ordinary shoulder and elbow offsets to remain visibly
subordinate to the head. It allows a larger explanatory body beat only when all
of these conditions hold:

- the drawing is associated with an eligible phrase peak, with at least 3.5
  seconds of planned performance time since the previous emphasis;
- its peak energy is at least 72% of the available waveform maximum;
- its drawing index is at least three greater than the preceding body beat
(at least two drawings intervene); and
- it would not immediately repeat an explanatory lean.

The full body beat uses the explanatory lean's approved arm shape beneath the
selected head stroke; it does not replace the nod with another lift.

This is the movement hierarchy:

```text
head direction and eyeline          primary on every phrase
shoulder/elbow follow-through       visible secondary action
full explanatory body beat          sparse emphasis only
speaking-energy light               supporting state cue
audio                               timing source and semantic content
```

A fresh speech onset from holding prepares with a 0.18-second head drawing
opposite the first lead at 25% of its amplitude. An executing thinking or idle
handover skips this preparation; streamed extensions never repeat it. Audio
starts without waiting for motion preparation. Completed audio with at least
two seconds of performance budget reserves a 0.28-second resolve drawing,
lifting head pitch by 0.035 rad toward the listener before the anchor return.
Provisional streams omit resolve, and finalization with at most 0.9 seconds
left retains its plain settle-only path.

The performance ends with one zero-offset `settle` around the pre-speech
anchor. Internal drawings are `through` except intentional quiet holds, which
use `settle` at a non-neutral phrase pose. There is no independent periodic
elbow oscillator and no scheduler gap between gestures.

### Policy values

These values are character policy, not hardware limits:

| Policy                                      | Implemented value                                     |
| ------------------------------------------- | ----------------------------------------------------- |
| Onset preparation                          | `0.18 s`, opposite head lead at `0.25 ×` amplitude      |
| Turn-yield resolve                          | `0.28 s`, `+0.035 rad` pitch when budget permits        |
| Idle head/yaw amplitude variation            | `0.85–1.15`; arm offsets unchanged                     |
| Idle travel tempo variation                 | `0.9–1.1`; glance hold `0.5–1.0 s`                      |
| Added idle head start                       | Nominal `0.08 s`; 12% intermediate head drawing        |
| Motion end lead before audio duration       | `0.12 s`                                              |
| Nominal final settle budget                 | `0.55 s`, bounded for short utterances                |
| Phrase-duration scale                       | `1.35`                                                |
| Ordinary phrase base duration               | `1.35 s` before scale, randomization, and style tempo |
| Emphasis phrase base duration               | `0.90 s` before scale, randomization, and style tempo |
| Emphasis eligibility spacing               | At least `3.5 s` of planned performance time          |
| Commanded emphasis apex lead                | `0.17 s`; compiled acceptance `0.08–0.25 s` before peak |
| Minimum preceding drawing                   | `0.35 s`                                             |
| Quiet hold eligibility                      | Quiet interval at least `0.4 s`; hold matches the gap, capped at `1.2 s` |
| Speech plan length                          | `20 s` per piece; up to `25 s` when that finishes the audio |
| Duration randomization                      | `0.90–1.10`                                           |
| Ordinary head amplitude multiplier          | `0.88–1.10`                                           |
| Emphasis head amplitude multiplier          | `1.05–1.24`                                           |
| Ordinary yaw turn                           | `0.045–0.085 rad`                                     |
| Emphasis yaw turn                           | `0.070–0.110 rad`                                     |
| Ordinary body multiplier on source drawing  | `0.32–0.48`                                           |
| Full body-beat multiplier on source drawing | `0.78–0.96`                                           |
| Head-lead share                             | Ordinary `0.64–0.74`; emphasis `0.25–0.35`              |
| Local loudness head multiplier              | `0.85 + 0.30 × smoothstep(mean RMS / maximum RMS)`      |
| Matching roll/yaw signs                     | 70% bias; reflective roll keeps its authored sign     |
| Next-head look-ahead during body follow     | `0.18`                                                |
| Full body-beat energy gate                  | At least `0.72` of available waveform maximum                  |
| Full body-beat spacing                      | Drawing-index difference of at least `3`              |

Seeded variation selects the value inside each range. The final compiled
motion may be uniformly reduced near calibration boundaries and may be retimed
to respect the motor-speed ceiling.

### Interruption and failure

Speech motion is best-effort: a movement-planning failure must not silence a
valid response. If movement cannot start, audio continues.

When playback ends before the generated movement, the runtime replaces the
executing spline with an anchor-relative settle from commanded position,
velocity and acceleration, preserving the motion run. A settle already installed by stream
finalization, or a performance waiting for measured settling, continues without
restarting. If spline replacement is unavailable,
the runtime falls back to stopping and settling from measured state.
Cancellation does the same. Position, velocity and acceleration are preserved at replacement when
calibration permits. Near a limit, safety protection halves the offending
joint's starting velocity and acceleration together; continuity yields to
calibration bounds. Measured starts use zero acceleration.
If audio upload, validation, synthesis, or playback
fails, the speech run becomes `failed`, the coordinator removes its temporary
file, and character motion returns to the anchor. Idle resumes only after the
scheduler chooses another randomized delay.

The voice coordinator's playback-complete acknowledgement may arrive while the physical
settle is still running. Listening, thinking, and neutral reactions preserve
the speaking or settling state until that movement is cleaned up. The next
speech waits only for a matching active movement; a run that has left the
runtime's bounded movement history cannot block a later performance.

### Thinking handover

Verification, endpointing and agent processing can all request thinking during
the same voice turn. Once the character is thinking, repeated notifications keep
the current movement and its timing. They can refresh attention expiry without
replaying the opening tilt. This prevents a processing-stage transition from
causing a sudden movement restart.

When speech takes over, the runtime starts from the commanded thinking position,
velocity and acceleration when available. Speech and its final settle retain priority over
later listening, thinking or neutral notifications. These rules live in
`CharacterCoordinator` and `RuntimeCore::extend_character_performance()`.
Tests cover repeated thinking requests and the handover into speech.

## Trajectory compiler

### Constants and authorities

| Concern | Authority |
| --- | --- |
| Command rate | `50 Hz` (`20 ms` period) in `RuntimeCore` and the daemon loop |
| Position limits | Active calibration loaded by the hardware driver |
| Offline position limits | `simulation/mujoco/config/servo_calibration.json` |
| Encoder resolution | `4096` counts/revolution |
| Motor speed ceiling | `5.445427266... rad/s`, the 7.4 V STS3215 52 RPM no-load specification |
| Completion position tolerance | `0.05 rad` maximum joint error |
| Completion velocity tolerance | `0.05 rad/s` maximum measured joint speed |
| Required settled duration | `0.25 s` continuously within both tolerances |
| Settling timeout | `2.0 s` after authored trajectory completion |
| Simulation reporting policy | `motion/config/stability_limits.yaml`; diagnostic, not a runtime gate |

The driver reports voltage, current, temperature, and status telemetry.
Position calibration and the motor profile remain the command authorities.

### Compilation entry points

#### One-target `goto`

`JointTrajectory::with_start_velocity_calibrated` wraps one target as a single
`settle` waypoint. It uses the same quintic compiler, speed retiming, measured
start velocity, and calibration-safe interruption path as a multi-keyframe
motion.

#### Authored motion

`MotionSequence::compile_scaled_calibrated`:

1. resolves every keyframe target from its absolute pose or relative anchor;
2. applies the one uniform relative amplitude scale;
3. converts keyframe arrival and marker data to `TrajectoryWaypoint` values;
4. invokes `CompiledTrajectory::compile_calibrated`; and
5. retains keyframe and marker queries for runtime and scene status.

#### Character-owned generated motion

`RuntimeCore::play_generated_anchored_relative` accepts a generated
`MotionDefinition` and applies the same contract as a loaded relative clip:

- motion space must be `anchor_relative`;
- `return_to_anchor` must be true;
- the final keyframe must be one zero-offset `settle`;
- every resolved target must pass driver validation; and
- compilation preserves the immutable anchor, uniform amplitude scale,
  calibration, and the STS3215 speed ceiling.

A fresh movement starts from measured position and velocity while the runtime
is holding. `extend_character_performance` replaces an executing character
movement from its commanded position and velocity, keeping the same run ID.
Speech uses this replacement path when taking over from thinking or idle,
extending a stream, and returning to the anchor at playback end. Preserving
the commanded state avoids restarting each continuation from delayed servo
feedback. Both paths use the same trajectory compiler and limits.

### Input normalization

The compiler requires:

- a non-empty name;
- one complete finite start position and start velocity for the same joints;
- at least one complete finite waypoint;
- positive finite movement durations;
- finite non-negative holds attached only to `settle` arrivals; and
- a positive finite speed ceiling.

Calibrated compilation additionally requires one unique, finite, ordered
`lower_rad < upper_rad` range for every joint and a start position inside each
range.

### Quintic segment model

For each joint and segment, the compiler constructs:

```text
p(t) = c0 + c1 t + c2 t² + c3 t³ + c4 t⁴ + c5 t⁵
```

The six coefficients satisfy six endpoint constraints:

```text
p(0) = p0       p(T) = p1
v(0) = v0       v(T) = v1
a(0) = a0       a(T) = a1
```

Because adjacent segments use the same waypoint position, velocity, and
acceleration, `through` boundaries are C2-continuous: position, first
derivative, and second derivative match at the join.

The compiler keeps exact target positions. It changes derivative values and
segment durations, not authored keyframe positions.

### Duration policy

The initial compiled duration for each authored segment is:

```text
through: authored_duration / style.tempo

settle:  authored_duration / style.tempo
         × (0.85 + 0.30 × style.settle_character)
```

A hold is appended after arrival and does not participate in polynomial
motion. Sampling during a hold returns the exact target with zero velocity and
acceleration.

Speed retiming may lengthen individual movement segments after this style
transformation.

### Internal derivatives

Let `before` and `after` be the average slopes of the adjacent authored
segments for one joint.

#### Start point

The caller supplies start position and velocity. Ordinary movements use the
latest measured position, clamped to calibration, and measured velocity.
Character performance replacements sample the executing trajectory's
commanded position, velocity and acceleration. Optional start acceleration
defaults to zero for measured starts. Replacements preserve all three values
when calibration permits. Protection halves the offending joint's velocity
and acceleration together if its curve leaves the calibrated range; safe
joints keep their supplied derivatives. This safety exception can break
continuity at a replacement near a limit.

#### Final point and settle arrivals

Velocity and acceleration are zero at the final waypoint and after every
`settle` arrival.

#### Through arrivals

If adjacent slopes change sign or either is zero, the waypoint velocity is
zero. This represents an instantaneous direction reversal, not a hold.

Otherwise the compiler calculates a duration-weighted slope, bounds its
magnitude to three times the smaller adjacent slope, and multiplies it by
style tangent tension and joint-lag character.

The compiler bases through acceleration on the change between adjacent slopes
over their combined duration, then scales it by tangent tension, joint-lag
character, and overshoot character.

Joint-lag character uses the ordered chain:

```text
base yaw → shoulder pitch → elbow pitch → head roll → head pitch
```

The configured lag reduces derivative magnitude progressively across that
order. It changes how joints travel through shared drawings; it does not delay
servo packets or change the 50 Hz clock. Explicit head-first timing, such as
speech body follow-through, is represented with additional semantic drawings.

### Unrequested overshoot control

For each segment and joint, the compiler samples the quintic at 80 subdivisions
and compares its position with the closed interval between the segment's two
authored endpoints.

If a polynomial exits that interval, the compiler halves only the velocity and
acceleration values bordering that segment and joint. It repeats this local
process up to eight passes.

Local attenuation matters. Flattening one joint's derivatives globally would
allow a difficult turn in one place to create stopped-looking movement in an
unrelated part of a long performance.

An authored overshoot pose remains an endpoint, and the trajectory reaches it
exactly. The overshoot guard prevents additional polynomial overshoot between
authored endpoints.

### Motor-speed retiming

After compiling a candidate, the runtime samples every segment's joint
velocity at 81 points. If a segment exceeds the STS3215 ceiling, only that
segment's duration is multiplied by:

```text
(measured_peak / maximum_velocity) × 1.015
```

The compiler rebuilds derivatives and polynomials and repeats for up to 12
iterations. It rejects the trajectory if the final peak remains more than
0.1% above the ceiling.

Retiming changes marker arrival times because markers belong to compiled
keyframes. Scenes ask the active motion whether a marker has been reached,
which keeps light and audio synchronized with the stretched motion.

### Calibration-safe interruption

The supplied start velocity shapes the transition into a movement. A high
velocity near a joint boundary can make the polynomial leave calibration;
noisy measured feedback can cause the same problem. `compile_calibrated`
reduces only the affected joints' starting velocities:

1. Validate that the start position is inside every range.
2. Bound any starting speed above the motor ceiling to 95% of that ceiling,
   preserving direction.
3. Compile the full candidate.
4. Sample it at the actual 50 Hz command rate.
5. Identify only joints whose samples leave calibration.
6. Halve only those joints' start velocities.
7. Recompile, for at most 18 iterations.

Safe joint velocities that do not cause a boundary violation remain intact.
Targets, styles, and other joints do not change. Failure to find a safe blend
is a rejected motion, never a clipped command.

## Servo control

### Calibration and radians conversion

Each joint calibration contains:

- semantic joint name;
- unique servo ID;
- raw encoder value corresponding to joint-space zero;
- encoder direction, `+1` or `-1`; and
- safe minimum and maximum raw deltas around neutral.

The software converts safe raw deltas to an ordered radian range using the
4096-count encoder. The driver converts a commanded radian value as follows:

```text
steps_per_radian = 4096 / (2π)
delta = round(radians × steps_per_radian) × encoder_direction
raw_goal = (neutral_raw + delta) modulo 4096
```

The driver rejects a delta outside its calibrated range before producing a raw
goal. It also requires a command for exactly all five joints.

Feedback conversion unwraps the raw value to the nearest signed half-turn from
neutral and divides by encoder direction and `steps_per_radian`. Orion's five
servos have Phase register 18 bit 2 set: Present Speed register 58 reports
encoder counts per second. The transport decodes the little-endian word as
sign magnitude, with bit 15 marking a negative value. The driver converts it
with one named constant:

```text
velocity_rad_s = signed_present_speed × (2π / 4096) × encoder_direction
```

The [Feetech magnetic-encoder memory table](https://www.feetechrc.com/Data/feetechrc/upload/file/20240702/舵机协议内存表-磁编码版本.xlsx)
defines the count/s unit and the Phase selection; clearing bit 2 instead selects
50 counts/s per unit and is incompatible with this conversion. The
[vendor SDK's ReadSpeed](https://github.com/ftservo/FTServo_Arduino/blob/main/src/SMS_STS.cpp)
confirms the sign encoding. The Orion profile preserves bit 2. This conversion
assumes the commissioned setting; the driver does not add a unit-mode check.

Fresh measured starts and final completion use this physical rad/s value.
Speech spline replacements use commanded derivatives, and MuJoCo already
reports rad/s. The hardware speed quantum is 50 counts/s, about 0.0767 rad/s,
despite the count/s unit. That exceeds
the unchanged 0.05 rad/s completion threshold, so these servos still need a
zero-speed reading throughout the settle window.

### Hardware preparation and torque activation

The STS3215 driver has three distinct stages.

#### Connect and validate

With torque off, it opens the serial port and verifies for every configured
servo:

- unique ID and joint name;
- STS3215 model number;
- torque disabled;
- zero fault status; and
- the same firmware version across all five devices.

#### Apply the Orion profile

The driver applies and reads back return delay, operating mode, direction,
PID coefficients, maximum acceleration, and runtime acceleration. Persistent
register writes unlock and relock EEPROM. Gravity-loaded joints use raised
proportional gains so they follow small animation offsets and settle inside
tolerance. V1: shoulder and elbow 32, head pitch 48. V2: shoulder and elbow 32.
All other joints use the factory gain of 16.

The servo acceleration registers shape the actuator's local response. They do
not replace the host trajectory or define a second motion plan.

#### Activate

Immediately before torque-on, the driver synchronously reads present state,
writes each present encoder position into its goal register, verifies those
goals, and only then enables torque. This ordering prevents torque activation
from snapping toward a stale target.

Deactivation disables torque on every configured ID. Dropping the hardware
driver also attempts torque-off before closing the serial port.

### Synchronized servo I/O

The physical transport uses the STS3215 serial protocol through `rustypot`.
One synchronized feedback read requests the 15-byte state block from all five
IDs. It decodes position, sign-magnitude velocity and current, voltage,
temperature, and status.

One synchronized goal write sends the five two-byte positions to goal register
address 42. The trajectory loop does not issue five independently timed
position writes.

Register-level operations remain private to `Sts3215Driver` and
`Sts3215Transport`. Network and character layers cannot address registers.

### The 50 Hz daemon cycle

The outer service loop advances a monotonic deadline by 20 ms each iteration.
Its order is:

1. `RuntimeCore::tick`
2. scene coordinator tick
3. speech coordinator tick
4. speaking-energy light update
5. character coordinator tick
6. background character light update when no foreground owner exists
7. pending Unix command handling
8. sleep until the next deadline

Inside `RuntimeCore::tick`:

```text
read synchronized feedback
        │
        ├─ active MotionSequence? ─ sample positions + markers
        │
        └─ active JointTrajectory? ─ sample positions
        │
write synchronized goals
update executing progress
        │
authored duration complete?
        ├─ no  → publish snapshot
        └─ yes → enter measured settling → publish snapshot
```

The clock is supplied to `RuntimeCore`, which makes lifecycle and trajectory
tests deterministic. Hardware uses `Instant`; tests can advance time directly.

### Movement lifecycle

```text
executing ── authored samples exhausted ──▶ settling
    │                                         │
    │ stop                                    ├─ stable within tolerance ─▶ completed
    ▼                                         └─ timeout ─────────────────▶ timed_out
cancelled
```

`executing` tracks the compiled trajectory. `settling` tracks physical
feedback against the final target. Completion requires the maximum absolute
joint error and maximum absolute measured velocity to remain within tolerance
for the full settle window.

The daemon returns to `holding` for every terminal movement phase. Disabling
torque is a separate command and cancels any active movement first.

### Marker and status behavior

`MotionSequence` exposes:

- active keyframe label and index;
- total keyframe count;
- progress through total compiled duration;
- compiled marker arrival times; and
- all markers reached at the sampled elapsed time.

Movement run IDs are daemon-local and reset at restart. Status retains only
the active run and the most recent terminal run. Clients must retain the
returned run ID and follow that specific ID; status is not an event database.

### Runtime validation and failure modes

| Boundary | Rejection or terminal behavior |
| --- | --- |
| Asset load | Malformed schema, unknown fields, invalid references, or duplicate names reject the catalog |
| Runtime startup/reload | Absolute targets outside active calibration reject the catalog transaction |
| Relative instantiation | Missing anchor joints, missing limits, or invalid final zero settle reject the clip |
| Trajectory compile | Invalid maps, durations, holds, limits, speed, overshoot, or safe interruption reject movement before start |
| Driver encode | Missing/non-finite/out-of-range commands reject before serial write |
| Movement execution | Driver read/write errors propagate as runtime errors; no completion is claimed |
| Settling | Failure to remain within measured tolerance becomes `timed_out` |
| Cancellation | Compiled trajectory is dropped and the run becomes `cancelled`; holding torque remains on |

### Shared hardware and simulation path

`RuntimeDriver` is expressed in calibrated joint radians:

```text
apply_servo_profile
activate / deactivate
read
write
joint_limits
validate_positions
clamp_positions_to_safe_range
```

The physical driver converts radians to STS3215 packets. `MujocoDriver`
exchanges radians with the simulator. Everything above that boundary—asset
loading, trajectory compilation, timing, markers, run IDs, cancellation, and
settling—uses the same Rust code.

The `orion-trajectory` binary also calls the same compiler to emit portable
50 Hz preview samples. Studio and offline tools consume those samples rather
than reimplementing interpolation.

### Verification map

| Property | Primary automated evidence |
| --- | --- |
| Complete schema and catalog | `pose`, `motion`, and `scene` loader tests |
| C2 continuity at `through` | `trajectory::keeps_position_velocity_and_acceleration_continuous_through_keyframes` |
| Exact settle derivatives | trajectory and motion-sequence sampling tests |
| Segment speed ceiling | `trajectory::retimes_fast_segments_to_the_sts3215_ceiling_without_extra_overshoot` |
| Measured interruption continuity | interruption trajectory tests |
| Calibration-safe interruption | calibrated interruption tests |
| Relative scaling and anchor return | built-in relative clip tests |
| 50 Hz movement lifecycle | daemon state-machine tests |
| Shared MuJoCo execution | native MuJoCo runtime test |
| Radians/raw and synchronized transport | driver and transport tests |

Automated checks establish the numerical and lifecycle contracts. Physical
review must also assess silhouette, perceived timing, mechanical sound, cable
behavior, and appeal using the [animation catalogue](#animation-catalogue).

### Export a trajectory

Generate a preview or diagnostic document with the same compiler:

```bash
runtime/target/release/orion-trajectory --hardware v2 \
  --motion look_at_left_expressive --start-pose attentive \
  --pose-file motion/config/v2/poses.yaml \
  --motions-directory motion/motions/v2 \
  --calibration ~/.config/orion/servo_calibration-v2.json
```

For an anchor-relative motion, add `--anchor-pose POSE_NAME`. The output holds
positions, velocities, accelerations, markers, calibration ranges and
hardware-profile metadata.
