# Motion asset reference

Orion loads one pose schema and one motion schema. Both use
`format_version: 2`. The loaders reject unknown fields, so copied examples do
not rely on ignored compatibility fields.

## Repository layout

```text
motion/
├── config/
│   ├── poses.yaml                  built-in complete poses
│   └── stability_limits.yaml       MuJoCo reporting policy only
├── user/poses/**/*.yaml            Studio-authored complete poses
└── motions/
    ├── expressive/*.yaml           character actions
    ├── functional/*.yaml           direct utility actions
    ├── idle/*.yaml                 anchor-relative ambient clips
    ├── speaking/*.yaml             source drawings for generated speech
    └── user/**/*.yaml              Studio-authored motions
```

The loaders traverse built-in and user files recursively in sorted path order.
Semantic names are global within each asset type. A user asset cannot shadow a
built-in or another user asset.

## Joint vocabulary

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

| Field | Required | Contract |
| --- | --- | --- |
| `format_version` | Yes | Integer `2` |
| `units` | No | If present, must be `radians` |
| `poses` | Yes | Non-empty mapping keyed by unique semantic names |
| `description` | No | Human-readable intent and silhouette |
| `tags` | No | Semantic-name list used for lifecycle and catalog policy |
| `idle_profile` | No | Semantic profile used by character idle selection |
| `default_lighting` | No | Must name a built-in lighting effect |
| `positions` | Yes | Exactly one finite radian value for every Orion joint |

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
- `calibration_reference` reserves `zero_reference` for commissioning.

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

| Field | Required | Contract |
| --- | --- | --- |
| `format_version` | Yes | Integer `2` |
| `motion` | Yes | One motion mapping |
| `name` | Yes | Unique semantic name |
| `description` | No | User-facing purpose and acting intent |
| `space` | Yes | `absolute` or `anchor_relative` |
| `style` | Yes | One named style from the table below |
| `return_to_anchor` | Relative only | Must be `true` for relative motion; prohibited for absolute motion |
| `keyframes` | Yes | Non-empty ordered list |

### Keyframe fields

| Field | Required | Contract |
| --- | --- | --- |
| `pose` | Absolute only | Existing named pose; `offsets` must be absent |
| `offsets` | Relative only | Partial finite joint map; `pose` must be absent; omitted joints mean zero |
| `duration` | Yes | Finite seconds greater than zero before style tempo is applied |
| `arrival` | Yes | `through` or `settle` |
| `hold` | No | Finite, non-negative seconds; greater than zero only with `settle` |
| `marker` | No | Unique semantic name reached at the compiled arrival time |

The final keyframe must use `settle`. The final relative keyframe must have no
non-zero offsets.

## Absolute and relative target resolution

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

Rust defines styles as compiled constants. They are artistic policy and contain
no calibration or motor limits.

| Style | Tempo | Tangent tension | Joint lag | Amplitude | Overshoot scale | Settle character | Intended use |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| `living_idle` | 0.82 | 0.38 | 0.18 | 0.90 | 0.00 | 0.85 | Unhurried low-amplitude ambient motion |
| `attentive` | 1.08 | 0.58 | 0.12 | 1.00 | 0.15 | 0.58 | Upward attentive entry and hold detail |
| `expressive_turn` | 1.00 | 0.72 | 0.22 | 1.00 | 1.00 | 0.62 | Anticipation, lean, authored overshoot, settle |
| `speaking_calm` | 0.72 | 0.42 | 0.16 | 0.95 | 0.00 | 0.82 | Restrained conversational source clips |
| `speaking_emphatic` | 1.12 | 0.62 | 0.12 | 1.00 | 0.18 | 0.62 | Generated utterance performance and phrase emphasis |
| `thinking` | 0.68 | 0.36 | 0.24 | 0.62 | 0.08 | 0.88 | Slow asymmetric thought |
| `quick_reaction` | 1.34 | 0.70 | 0.08 | 0.92 | 0.24 | 0.48 | Short decisive acknowledgement |
| `return_home` | 0.74 | 0.32 | 0.20 | 1.00 | 0.00 | 1.00 | Weighted final return |

Interpretation:

- A higher `tempo` shortens authored segment duration.
- `tangent_tension` scales internal derivative energy.
- `joint_lag` changes derivative character across the ordered joint chain; it
  is not a separate scheduler delay.
- `amplitude` scales anchor-relative offsets.
- `overshoot_scale` affects internal acceleration character; it never creates
  permission to leave the interval between authored segment endpoints.
- `settle_character` changes the timing weight of settle segments.

## Built-in motion catalog

### Expressive

- `look_at_left_expressive`
- `look_at_right_expressive`
- `attention_left`
- `attention_right`
- `attentive_entry`
- `acknowledge_nod`
- `disagree_soft`
- `curious_tilt`
- `delight_lift`
- `thinking_shift`

### Functional

- `look_at_left`
- `look_at_right`
- `return_home`

### Idle

- `idle_breathe`
- `idle_head_curiosity`
- `idle_micro_glance`
- `idle_shoulder_adjust`
- `idle_weight_shift`
- `idle_soft_head_shake`
- `idle_attentive_hold`
- `idle_directional_hold`

### Speaking source drawings

- `speak_calm_sway`
- `speak_emphasis_nod`
- `speak_explanatory_lean`
- `speak_reflective_tilt`

The runtime-generated `speaking_performance` and interruption-only
`speak_settle` are not YAML assets. `CharacterCoordinator` constructs them in
memory from waveform analysis and the approved speaking source drawings.

## Validation ownership

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
