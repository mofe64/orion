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

## Music movement

Orion dances around the current anchor pose. The shoulder and elbow lower the body on each beat, then lift it with a softer return. The head moves with the body.

The base sways from side to side across a musical phrase. The neck starts each turn before the base. A small opposite movement prepares the turn. The neck moves past its target, then settles. Head pitch adds downward accents on selected strong beats.

Quiet music produces a gentle sway and small breathing movements. Stronger music adds body bounces, side movements, looking turns and a larger phrase ending. Orion selects a movement pattern at a phrase boundary. It blends the patterns while the dance continues.

The dance blends from the current pose over a beat. When the dance ends, Orion returns to the anchor with a small final movement. Speech and tracking take control from the current movement. Dance targets flow through their intermediate positions. One scale fits the drawing inside joint limits and the desk comfort box.

`Off` disables dancing. `Sometimes` alternates dance stretches with ordinary idle movement. `Always` permits dancing while music plays. Dancing starts at a phrase boundary after the beat tracker finds the beat. Speech, tracking and foreground actions keep their existing priority.

The beat tracker gives kick and bass attacks more weight than higher sounds. It compares each rhythm with half and double its tempo. Similar rhythms favor a moderate dance tempo. Ambiguous fast drum rhythms use half time.

A locked tempo changes only after a stronger pattern continues for several seconds. The beat counter continues through the change.

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

Activation requires valid status, available feedback and checked initial goal writes. It enables torque only after those checks. The zeroing command keeps torque off and restores previous offsets after failed verification.

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

V2 uses the motions below. [V1 motion files](../motion/motions/v1/) remain separate.

| Motion | Intent | File |
| --- | --- | --- |
| `arrival_notice` | Raise the head first, then adopt the attentive posture and return. | [V2](../motion/motions/v2/arrival_notice.yaml) |
| `attention_left` | A small grounded turn toward the confirmed speaker on the left. | [V2](../motion/motions/v2/attention_left.yaml) |
| `attention_right` | A small grounded turn toward the confirmed speaker on the right. | [V2](../motion/motions/v2/attention_right.yaml) |
| `delight_lift` | Lift the head and look up into the delight posture. | [V2](../motion/motions/v2/delight_lift.yaml) |
| `idle_attentive_hold` | Raise the gaze slowly while holding an attentive posture. | [V2](../motion/motions/v2/idle_attentive_hold.yaml) |
| `idle_breathe` | Raise the head slowly, breathe through the arm, and settle back. | [V2](../motion/motions/v2/idle_breathe.yaml) |
| `idle_directional_hold` | Raise the gaze slowly while holding a directional or desk anchor. | [V2](../motion/motions/v2/idle_directional_hold.yaml) |
| `idle_head_curiosity` | Raise the gaze and turn the neck slowly to inspect the surroundings. | [V2](../motion/motions/v2/idle_head_curiosity.yaml) |
| `idle_micro_glance` | Turn the base slowly for a brief glance with a full return. | [V2](../motion/motions/v2/idle_micro_glance.yaml) |
| `idle_shoulder_adjust` | Redistribute the shoulder and elbow slowly, then return. | [V2](../motion/motions/v2/idle_shoulder_adjust.yaml) |
| `idle_soft_head_shake` | Turn the neck slowly left and right with a soft return. | [V2](../motion/motions/v2/idle_soft_head_shake.yaml) |
| `idle_weight_shift` | Shift the shoulder and elbow slowly, then restore their balance. | [V2](../motion/motions/v2/idle_weight_shift.yaml) |
| `look_around` | Deliberate three-stop desk search, neck leading the base, then returning to its anchor. | [V2](../motion/motions/v2/look_around.yaml) |
| `look_at_left` | Turn directly toward the predefined target on Orion's left. | [V2](../motion/motions/v2/look_at_left.yaml) |
| `look_at_left_expressive` | Notice, lean toward, and settle on the predefined left target. | [V2](../motion/motions/v2/look_at_left_expressive.yaml) |
| `look_at_right` | Turn directly toward the predefined target on Orion's right. | [V2](../motion/motions/v2/look_at_right.yaml) |
| `look_at_right_expressive` | Notice, lean toward, and settle on the predefined right target. | [V2](../motion/motions/v2/look_at_right_expressive.yaml) |
| `mutual_gaze_away` | Turn the neck aside with a raised gaze, then return. | [V2](../motion/motions/v2/mutual_gaze_away.yaml) |
| `mutual_gaze_hi` | Raise the gaze in a clear head-only greeting and return. | [V2](../motion/motions/v2/mutual_gaze_hi.yaml) |
| `mutual_gaze_notice` | Raise the gaze, turn the neck with curiosity, and return. | [V2](../motion/motions/v2/mutual_gaze_notice.yaml) |
| `peek_new_object` | Arm-led desk peek with a curious neck turn and a full anchor return. | [V2](../motion/motions/v2/peek_new_object.yaml) |
| `pet_notice` | Lift into delight, follow the pet briefly, and return to the held anchor. | [V2](../motion/motions/v2/pet_notice.yaml) |
| `return_home` | Move from the current joint state to Orion's home pose. | [V2](../motion/motions/v2/return_home.yaml) |
| `speak_calm_sway` | Sway the neck and arm gently with alternating raised and lowered gaze. | [V2](../motion/motions/v2/speak_calm_sway.yaml) |
| `speak_emphasis_nod` | Nod down at a phrase boundary, lift back, and settle at the anchor. | [V2](../motion/motions/v2/speak_emphasis_nod.yaml) |
| `speak_explanatory_lean` | Lean forward with a raised gaze for explanatory emphasis, then return. | [V2](../motion/motions/v2/speak_explanatory_lean.yaml) |
| `speak_react_agree` | Make two clear downward nods with an upward return between them. | [V2](../motion/motions/v2/speak_react_agree.yaml) |
| `speak_react_curious` | Raise the gaze and turn the neck with a slight upright lean. | [V2](../motion/motions/v2/speak_react_curious.yaml) |
| `speak_react_disagree` | A clear three-beat head shake used as a spoken disagreement reaction. | [V2](../motion/motions/v2/speak_react_disagree.yaml) |
| `speak_react_happy` | Lift the head and gaze with two bright upward bounces. | [V2](../motion/motions/v2/speak_react_happy.yaml) |
| `speak_react_laugh` | Make three raised head bounces with a light arm lift. | [V2](../motion/motions/v2/speak_react_laugh.yaml) |
| `speak_react_surprised` | Pull the arm back and raise the head for a surprised pause. | [V2](../motion/motions/v2/speak_react_surprised.yaml) |
| `speak_react_sympathy` | Lower the head and turn the neck softly for sympathy. | [V2](../motion/motions/v2/speak_react_sympathy.yaml) |
| `speak_react_sorry` | Shrink back and lower the head after a mistake. | [V2](../motion/motions/v2/speak_react_sorry.yaml) |
| `speak_react_shy` | Turn away and lower the head, peek back, then return. | [V2](../motion/motions/v2/speak_react_shy.yaml) |
| `speak_react_thinking` | Look aside and slightly up during a thoughtful pause. | [V2](../motion/motions/v2/speak_react_thinking.yaml) |
| `speak_react_unsure` | Look slightly down while turning the neck from side to side. | [V2](../motion/motions/v2/speak_react_unsure.yaml) |
| `speak_reflective_tilt` | Turn the neck aside and lower the gaze, then return quietly. | [V2](../motion/motions/v2/speak_reflective_tilt.yaml) |

V2 poses include home, rest, attentive, delight, desk-work and desk-reach postures. Left and right look targets include anticipation, lean and overshoot poses. Exact angles remain in [poses.yaml](../motion/config/v2/poses.yaml).

Positive V2 head pitch lowers the gaze. Positive neck swivel turns the head right. V2 `head_roll_joint` names the neck swivel; it does not provide a sideways head tilt.

The [intent table](../motion/config/v2/movement-intents.yaml) defines movement direction, main keyframes, drawing sizes and preview contexts. A preview context identifies the starting pose and the anchor used for the return.

The Hands preview starts from home and follows scripted bearings. It includes raised hands, target loss and the held-pose return. Arrival, pet and object-peek previews call their runtime planners with representative observation targets.

The preview reports head height and gaze pitch relative to the starting pose. Positive gaze change means looking up. Operating targets retain a three-degree joint-limit margin. Rest and startup use the intentional mechanical-rest boundary.

Look targets keep combined base yaw and neck swivel within 90 degrees.

1. Compile the catalogue and run the movement checks without recording videos.

   ```bash
   .venv/bin/python scripts/preview_movements.py --validate-only --output ../scratch/motion-checks
   ```

2. Generate the videos and review index when required.

   ```bash
   .venv/bin/python scripts/preview_movements.py
   ```
