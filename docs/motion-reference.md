# Motion reference

The Rust runtime loads pose and motion files with `format_version: 2`. Joint values use radians. Unknown fields, duplicate names and invalid targets cause loading errors.

## Asset files

| Path | Content |
| --- | --- |
| `motion/config/<version>/poses.yaml` | Built-in complete poses |
| `motion/user/poses/<version>/` | User pose files |
| `motion/motions/<version>/` | Built-in motions |
| `motion/motions/<version>/user/` | User motions |
| `scenes/<version>/` | Built-in scenes and a `user/` subdirectory |

Versions are `v1` and `v2`. User names cannot replace built-ins. The runtime validates the complete catalogue before swapping loaded assets.

## Joint vocabulary and limits

Each complete pose contains `base_yaw_joint`, `shoulder_pitch_joint`, `elbow_pitch_joint`, `head_roll_joint` and `head_pitch_joint`.

This is the canonical export order. All numeric positions must be finite. Asset names contain ASCII letters, digits, hyphens or underscores.

V2 uses mechanical-rest coordinates and the limits in [v2-joints.json](../hardware/profiles/v2-joints.json). V1 uses servo calibration. [Hardware setup](hardware-versions.md#joint-names-and-limits) defines servo IDs and conversion.

Model geometry does not replace hardware command limits. Simulator stability settings do not grant additional travel.

## Pose schema

This example defines a complete V2 rest pose:

```yaml
format_version: 2
units: radians
poses:
  rest:
    description: Folded mechanical rest.
    tags: [rest]
    positions:
      base_yaw_joint: 0.0
      shoulder_pitch_joint: 0.0
      elbow_pitch_joint: 0.0
      head_roll_joint: 0.0
      head_pitch_joint: 0.0
```

| Pose field | Meaning |
| --- | --- |
| `description` | Human-readable text |
| `tags` | Role labels |
| `idle_profile` | Optional idle selection |
| `default_lighting` | Optional named light effect |
| `positions` | All five named joint angles |

`powered` and `idle_anchor` tags describe operating roles. `rest` and reference poses do not imply normal powered idle use. The selected pose file supplies exact targets.

## Motion schema

This absolute motion returns to a named pose:

```yaml
format_version: 2
motion:
  name: example_return
  description: Return to home.
  space: absolute
  style: return_home
  keyframes:
    - pose: home
      duration: 2.0
      arrival: settle
      marker: home_arrival
```

This relative motion moves around its anchor:

```yaml
format_version: 2
motion:
  name: example_nod
  description: Small head movement.
  space: anchor_relative
  style: speaking_calm
  return_to_anchor: true
  keyframes:
    - offsets: {head_pitch_joint: 0.03}
      duration: 0.8
      arrival: through
    - offsets: {}
      duration: 0.8
      arrival: settle
```

An anchor is the complete pose around which relative motion moves. Missing offset keys mean zero offset. Absolute frames use `pose`; relative frames use `offsets`.

| Motion field | Meaning |
| --- | --- |
| `name` | Unique semantic name |
| `description` | Human-readable text |
| `space` | `absolute` or `anchor_relative` |
| `style` | Named timing and shape preset |
| `return_to_anchor` | Relative motion's final return contract |
| `keyframes` | Ordered targets |

| Keyframe field | Meaning |
| --- | --- |
| `pose` | Named absolute pose |
| `offsets` | Relative joint offsets |
| `duration` | Positive travel time in seconds |
| `arrival` | `through` or `settle` |
| `hold` | Nonnegative hold after a settle arrival |
| `marker` | Optional unique event name |

`through` preserves movement across a target. `settle` sets final velocity and acceleration to zero. The final target settles. A hold cannot follow a through arrival.

## Motion styles

The presets are `living_idle`, `attentive`, `expressive_turn`, `speaking_calm`, `speaking_emphatic`, `thinking`, `quick_reaction` and `return_home`.

| Style property | Effect |
| --- | --- |
| `tempo` | Divides authored travel time |
| `tangent_tension` | Scales through-keyframe derivatives |
| `joint_lag` | Changes derivative character across joints |
| `amplitude` | Scales relative offsets |
| `overshoot_scale` | Changes internal acceleration shaping |
| `settle_character` | Adjusts settle travel time |

`joint_lag` does not insert a literal delay. `overshoot_scale` does not specify an angle. Exact preset values remain in [style.rs](../runtime/src/motion/style.rs).

## Target resolution

Absolute motion uses pose angles directly. Relative targets use this relation:

```text
resolved = anchor + style.amplitude × runtime_scale × offsets
```

One runtime scale fits the complete relative clip inside limits. Absolute targets do not receive amplitude scaling. An invalid absolute target fails.

## Trajectory compiler

The compiler connects targets with quintic segments. A quintic segment is a fifth-degree polynomial with position, velocity and acceleration boundary conditions.

Its position relation is:

```text
p(t) = c0 + c1t + c2t² + c3t³ + c4t⁴ + c5t⁵
```

Ordinary moves start from measured position and velocity. Streaming speech replacement starts from commanded position, velocity and acceleration. Internal through targets share derivatives. Settle targets end with zero derivatives.

The compiler limits unintended overshoot. It lengthens segments that exceed the STS3215 speed ceiling. It checks sampled positions against joint limits at 50 Hz.

Relative animation preserves its anchor. Interruption can reduce unsafe start derivatives while preserving a valid start position. Generated movement uses the same compiler as authored assets.

### Export a trajectory

Run these steps from the repository root.

1. Build the release compiler.

   ```bash
   cargo build --release --manifest-path runtime/Cargo.toml --locked
   ```

2. Export a V2 motion as JSON.

   ```bash
   runtime/target/release/orion-trajectory --hardware v2      --motion look_at_left_expressive --start-pose home      --pose-file motion/config/v2/poses.yaml --motions-directory motion/motions/v2      --calibration hardware/profiles/v2-joints.json
   ```

The output contains joint names, ranges, adjusted duration, markers and samples. Each sample includes position, velocity, acceleration and keyframe information.

`--anchor-pose` selects a relative anchor. `--start-state` reads JSON with complete `positions`, `velocities` and `anchor` maps. `--control-rate-hz` selects the export sample rate.

The export rate must be greater than zero and at most 1000 Hz. Runtime control remains at 50 Hz.

## Bearing gaze tuning

V2 serving enables gaze by default. Other operations and V1 leave it off. The planner uses these values from [gaze.rs](../runtime/src/expression/gaze.rs):

| Setting | Value |
| --- | --- |
| Target deadband | 4 degrees |
| Maximum extension frequency | 10 Hz |
| Neck lead | 200 ms |
| Compiled peak speed | 1.5 rad/s |
| Neck planning speed | 2.5 rad/s |
| Base planning speed | 0.6 rad/s |
| Maximum target age | 1 second |

These settings change gaze planning without changing profile limits. Unreachable bearings select a permitted boundary. See [gaze recovery](../runtime/README.md#bearing-gaze).

## Servo control

V2 command conversion uses the tracked direction and rest zero:

```text
raw_goal = 2047 + round(angle × 4096 / 2π) × direction
```

The driver rejects command ranges crossing raw 0/4095. Startup permits readings up to 20 counts outside profile limits. Initial goals clamp into range before torque activation. Later commands remain inside strict limits.

Activation requires valid status, available feedback and verified initial goal writes. It enables torque only after those checks. The zeroing command keeps torque off and restores previous offsets after failed verification.

Present speed converts from signed counts per second:

```text
velocity_rad_s = present_speed × 2π / 4096 × direction
```

This conversion requires the configured servo Phase bit. It does not imply one-count measurement precision.

## Runtime cycle and completion

```text
read synchronized feedback
          ↓
sample active trajectory and markers
          ↓
write synchronized goals
          ↓
update progress and settling state
```

The cycle runs every 20 ms. Timeline completion enters settling. Stable measured position and velocity permit completion. Timeout produces `timed_out`; cancellation produces `cancelled`.

Markers follow adjusted compiler timing. Status includes run identity, current keyframe and terminal outcome. Clients must distinguish timeline progress from measured completion.

## Animation catalogue

The table links to authored motion files. V1 and V2 motions with the same name use their own hardware assets.

| Motion | Space | Files |
| --- | --- | --- |
| `acknowledge_nod` | `anchor_relative` | [v1](../motion/motions/v1/expressive/acknowledge_nod.yaml), [v2](../motion/motions/v2/acknowledge_nod.yaml) |
| `arrival_notice` | `anchor_relative` | [v2](../motion/motions/v2/arrival_notice.yaml) |
| `attention_left` | `absolute` | [v1](../motion/motions/v1/expressive/attention_left.yaml), [v2](../motion/motions/v2/attention_left.yaml) |
| `attention_right` | `absolute` | [v1](../motion/motions/v1/expressive/attention_right.yaml), [v2](../motion/motions/v2/attention_right.yaml) |
| `attentive_entry` | `absolute` | [v1](../motion/motions/v1/expressive/attentive_entry.yaml), [v2](../motion/motions/v2/attentive_entry.yaml) |
| `curious_tilt` | `absolute` | [v1](../motion/motions/v1/expressive/curious_tilt.yaml), [v2](../motion/motions/v2/curious_tilt.yaml) |
| `delight_lift` | `absolute` | [v1](../motion/motions/v1/expressive/delight_lift.yaml), [v2](../motion/motions/v2/delight_lift.yaml) |
| `disagree_soft` | `anchor_relative` | [v1](../motion/motions/v1/expressive/disagree_soft.yaml), [v2](../motion/motions/v2/disagree_soft.yaml) |
| `hands_entry` | `absolute` | [v2](../motion/motions/v2/hands_entry.yaml) |
| `hands_return` | `anchor_relative` | [v2](../motion/motions/v2/hands_return.yaml) |
| `idle_attentive_hold` | `anchor_relative` | [v1](../motion/motions/v1/idle/idle_attentive_hold.yaml), [v2](../motion/motions/v2/idle_attentive_hold.yaml) |
| `idle_breathe` | `anchor_relative` | [v1](../motion/motions/v1/idle/idle_breathe.yaml), [v2](../motion/motions/v2/idle_breathe.yaml) |
| `idle_directional_hold` | `anchor_relative` | [v1](../motion/motions/v1/idle/idle_directional_hold.yaml), [v2](../motion/motions/v2/idle_directional_hold.yaml) |
| `idle_head_curiosity` | `anchor_relative` | [v1](../motion/motions/v1/idle/idle_head_curiosity.yaml), [v2](../motion/motions/v2/idle_head_curiosity.yaml) |
| `idle_micro_glance` | `anchor_relative` | [v1](../motion/motions/v1/idle/idle_micro_glance.yaml), [v2](../motion/motions/v2/idle_micro_glance.yaml) |
| `idle_shoulder_adjust` | `anchor_relative` | [v1](../motion/motions/v1/idle/idle_shoulder_adjust.yaml), [v2](../motion/motions/v2/idle_shoulder_adjust.yaml) |
| `idle_soft_head_shake` | `anchor_relative` | [v1](../motion/motions/v1/idle/idle_soft_head_shake.yaml), [v2](../motion/motions/v2/idle_soft_head_shake.yaml) |
| `idle_weight_shift` | `anchor_relative` | [v1](../motion/motions/v1/idle/idle_weight_shift.yaml), [v2](../motion/motions/v2/idle_weight_shift.yaml) |
| `look_around` | `anchor_relative` | [v2](../motion/motions/v2/look_around.yaml) |
| `look_at_left` | `absolute` | [v1](../motion/motions/v1/functional/look_at_left.yaml), [v2](../motion/motions/v2/look_at_left.yaml) |
| `look_at_left_expressive` | `absolute` | [v1](../motion/motions/v1/expressive/look_at_left_expressive.yaml), [v2](../motion/motions/v2/look_at_left_expressive.yaml) |
| `look_at_right` | `absolute` | [v1](../motion/motions/v1/functional/look_at_right.yaml), [v2](../motion/motions/v2/look_at_right.yaml) |
| `look_at_right_expressive` | `absolute` | [v1](../motion/motions/v1/expressive/look_at_right_expressive.yaml), [v2](../motion/motions/v2/look_at_right_expressive.yaml) |
| `mutual_gaze_away` | `anchor_relative` | [v2](../motion/motions/v2/mutual_gaze_away.yaml) |
| `mutual_gaze_hi` | `anchor_relative` | [v2](../motion/motions/v2/mutual_gaze_hi.yaml) |
| `mutual_gaze_notice` | `anchor_relative` | [v2](../motion/motions/v2/mutual_gaze_notice.yaml) |
| `peek_new_object` | `anchor_relative` | [v2](../motion/motions/v2/peek_new_object.yaml) |
| `pet_notice` | `anchor_relative` | [v2](../motion/motions/v2/pet_notice.yaml) |
| `return_home` | `absolute` | [v1](../motion/motions/v1/functional/return_home.yaml), [v2](../motion/motions/v2/return_home.yaml) |
| `speak_calm_sway` | `anchor_relative` | [v1](../motion/motions/v1/speaking/speak_calm_sway.yaml), [v2](../motion/motions/v2/speak_calm_sway.yaml) |
| `speak_emphasis_nod` | `anchor_relative` | [v1](../motion/motions/v1/speaking/speak_emphasis_nod.yaml), [v2](../motion/motions/v2/speak_emphasis_nod.yaml) |
| `speak_explanatory_lean` | `anchor_relative` | [v1](../motion/motions/v1/speaking/speak_explanatory_lean.yaml), [v2](../motion/motions/v2/speak_explanatory_lean.yaml) |
| `speak_react_agree` | `anchor_relative` | [v2](../motion/motions/v2/speak_react_agree.yaml) |
| `speak_react_curious` | `anchor_relative` | [v2](../motion/motions/v2/speak_react_curious.yaml) |
| `speak_react_disagree` | `anchor_relative` | [v2](../motion/motions/v2/speak_react_disagree.yaml) |
| `speak_react_happy` | `anchor_relative` | [v2](../motion/motions/v2/speak_react_happy.yaml) |
| `speak_react_laugh` | `anchor_relative` | [v2](../motion/motions/v2/speak_react_laugh.yaml) |
| `speak_react_surprised` | `anchor_relative` | [v2](../motion/motions/v2/speak_react_surprised.yaml) |
| `speak_react_sympathy` | `anchor_relative` | [v2](../motion/motions/v2/speak_react_sympathy.yaml) |
| `speak_react_thinking` | `anchor_relative` | [v2](../motion/motions/v2/speak_react_thinking.yaml) |
| `speak_react_unsure` | `anchor_relative` | [v2](../motion/motions/v2/speak_react_unsure.yaml) |
| `speak_reflective_tilt` | `anchor_relative` | [v1](../motion/motions/v1/speaking/speak_reflective_tilt.yaml), [v2](../motion/motions/v2/speak_reflective_tilt.yaml) |
| `thinking_shift` | `absolute` | [v1](../motion/motions/v1/expressive/thinking_shift.yaml), [v2](../motion/motions/v2/thinking_shift.yaml) |

V2 poses include operating anchors, rest, direction targets, desk-work poses and low desk reaches. Exact angles remain in [poses.yaml](../motion/config/v2/poses.yaml).

Generated speaker attention and speech performance are runtime plans rather than standalone authored files. Speech reactions require one apex marker and a return to the anchor.

See [motion ownership](motion-architecture.md), [speech animation](speech-animation-runtime.md) and [scene schema](../scenes/README.md).
