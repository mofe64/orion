# Motion reference

The Rust runtime loads pose and motion files marked `format_version: 2`, using radians for all joint angles. Unknown fields, duplicate names or out-of-range targets cause loading errors.

## Asset files

| Path | Content |
| --- | --- |
| `motion/config/<version>/poses.yaml` | Built-in complete poses |
| `motion/user/poses/<version>/` | User pose files |
| `motion/motions/<version>/` | Built-in motions |
| `motion/motions/<version>/user/` | User motions |
| `scenes/<version>/` | Built-in scenes and a `user/` subdirectory |

Versions are `v1` and `v2`. User-authored names cannot shadow built-in names, and the runtime validates the entire catalogue before swapping loaded assets.

## Joint vocabulary and limits

Every complete pose specifies all five joints in canonical export order: `base_yaw_joint`, `shoulder_pitch_joint`, `elbow_pitch_joint`, `head_roll_joint` and `head_pitch_joint`. All numeric positions must be finite, and asset names may contain only ASCII letters, digits, hyphens and underscores.

V2 uses mechanical-rest coordinates and limits from [v2-joints.json](../hardware/profiles/v2-joints.json); V1 uses user servo calibration. See [hardware setup](hardware-versions.md#joint-names-and-limits) for servo IDs and unit conversions. Simulator geometry and stability settings never override hardware command limits.

## Pose schema

Here is the V2 `rest` pose:

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
| `description` | Human-readable summary |
| `tags` | Role labels (such as `powered`, `idle_anchor` or `rest`) |
| `idle_profile` | Optional idle posture tag |
| `default_lighting` | Optional named light effect |
| `positions` | All five joint angles in radians |

The `powered` and `idle_anchor` tags indicate postures safe for sustained powered holding; `rest` and reference poses are not used for powered idle.

## Motion schema

Absolute motions travel to named poses:

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

Relative motions add offsets around whichever anchor pose Orion is currently holding:

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

Missing keys in `offsets` default to `0.0`. `through` arrivals keep moving smoothly across the waypoint, whereas `settle` arrivals stop with zero velocity and acceleration. The final keyframe must always use `settle`, and `hold` pauses are allowed only after a `settle` arrival.

| Motion field | Meaning |
| --- | --- |
| `name` | Unique motion identifier |
| `description` | Human-readable summary |
| `space` | `absolute` or `anchor_relative` |
| `style` | Named timing and shaping preset |
| `return_to_anchor` | Requires relative motions to finish back at their anchor |
| `keyframes` | Ordered list of target keyframes |

| Keyframe field | Meaning |
| --- | --- |
| `pose` | Named absolute pose (when `space: absolute`) |
| `offsets` | Joint offsets in radians (when `space: anchor_relative`) |
| `duration` | Positive travel time in seconds |
| `arrival` | `through` or `settle` |
| `hold` | Non-negative pause in seconds after a `settle` arrival |
| `marker` | Optional unique event name for lights or sound cues |

## Motion styles

Every motion selects one of eight styles (`living_idle`, `attentive`, `expressive_turn`, `speaking_calm`, `speaking_emphatic`, `thinking`, `quick_reaction` and `return_home`), letting the same keyframes feel relaxed, crisp or deliberate without rewriting them:

| Style property | What it changes |
| --- | --- |
| `tempo` | Overall speed (higher tempo shortens authored durations) |
| `tangent_tension` | How strongly Orion carries speed through `through` keyframes |
| `joint_lag` | How much joints move slightly out of step rather than peaking together |
| `amplitude` | Overall size of `anchor_relative` offsets |
| `overshoot_scale` | Acceleration shaping that controls how eagerly a move approaches its target |
| `settle_character` | Extra time given to settling softly at the end of a move |

Exact preset constants live in [style.rs](../runtime/src/motion/style.rs).

## Target resolution

Absolute keyframes resolve directly to their named pose angles, and an out-of-range absolute target is rejected. Relative keyframes resolve around the current anchor as:

```text
resolved = anchor + style.amplitude × runtime_scale × offsets
```

When a relative gesture would hit a joint limit, the compiler picks a single `runtime_scale` (`0–1`) for the whole clip so every joint stays inside limits while preserving the gesture's proportions.

## Music movement

When dancing to music, Orion moves around its current anchor: the shoulder and elbow lower on each beat and lift back softly, the base sways across musical phrases with the neck leading and slightly overshooting before settling, and head pitch adds downward accents on strong beats. Quiet music produces gentle breathing sways, while energetic passages bring in bounces, side turns and larger phrase endings blended smoothly at phrase boundaries.

Everything is scaled inside joint limits and the desk comfort box, and higher-priority activities (speech, tracking, foreground scenes) take control immediately.

- **`Off`:** Keeps ordinary idle movements while music plays.
- **`Sometimes` (default):** Alternates 20–40 second dance stretches with 1–3 minute breaks.
- **`Always`:** Dances whenever the beat tracker detects a clear beat.

The beat tracker weights kick and bass attacks above higher frequencies, checks half- and double-time candidates to prefer natural dance tempos, and switches tempo only after a stronger pattern persists for several seconds.

## Trajectory compiler

The compiler connects keyframes with quintic polynomials (`p(t) = c0 + c1t + c2t² + c3t³ + c4t⁴ + c5t⁵`), giving continuous position, velocity and acceleration. Ordinary motions start from measured position and velocity, while streaming speech extensions start from the active commanded state.

Before accepting a trajectory, the compiler bounds unintended overshoot, lengthens any segment that exceeds the STS3215 speed ceiling, and checks every 50 Hz sample against joint limits.

### Export a trajectory

Run these commands from the repository root to inspect a compiled trajectory as JSON:

1. Build the release compiler.

   ```bash
   cargo build --release --manifest-path runtime/Cargo.toml --locked
   ```

2. Export a V2 motion.

   ```bash
   runtime/target/release/orion-trajectory --hardware v2 \
     --motion look_at_left_expressive --start-pose home \
     --pose-file motion/config/v2/poses.yaml \
     --motions-directory motion/motions/v2 \
     --calibration hardware/profiles/v2-joints.json
   ```

The JSON output includes joint names, limits, adjusted duration, marker timestamps and samples (`positions`, `velocities`, `accelerations`, `keyframe_index`). Use `--anchor-pose` for a relative anchor, `--start-state` to pass an initial JSON state, and `--control-rate-hz` (up to `1000` Hz; runtime control remains `50` Hz) to change the sampling rate.

## Bearing gaze tuning

V2 serving enables bearing gaze by default (V1 and manual maintenance runs leave it off). The planner in [gaze.rs](../runtime/src/expression/gaze.rs) uses these parameters:

| Setting | Value |
| --- | --- |
| Target deadband | 4 degrees |
| Maximum extension frequency | 10 Hz |
| Neck lead | 200 ms |
| Compiled peak speed | 1.5 rad/s |
| Neck planning speed | 2.5 rad/s |
| Base planning speed | 0.6 rad/s |
| Maximum target age | 1 second |

Unreachable bearings clamp to the closest permitted boundary. See [bearing gaze](../runtime/README.md#bearing-gaze) for recovery details.

## Servo control

V2 converts radians to raw STS3215 goal counts (`0–4095`) using each joint's profile direction and rest zero (`2047`):

```text
raw_goal = 2047 + round(angle × 4096 / 2π) × direction
```

The driver rejects calibrated ranges that cross the `0/4095` wrap-around boundary. At startup, readings up to 20 counts outside profile limits are accepted and clamped into range before enabling torque; subsequent commands obey strict limits. Enabling torque requires valid servo status, live feedback and verified initial goal writes, while `oriond servo zero` keeps torque off and restores previous offsets if verification fails.

Present speed reads back as signed counts per second:

```text
velocity_rad_s = present_speed × 2π / 4096 × direction
```

This requires the configured servo `Phase` register bit and is subject to encoder quantization.

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

Each 20 ms tick reads feedback, samples the active trajectory and fires any reached markers (at the compiler's adjusted timing), then writes synchronized servo goals. When the timeline finishes, the run enters `settling` and waits for measured positions and velocities to stabilize inside tolerance before reporting `completed` (or `timed_out` / `cancelled`). Clients should track measured completion rather than assuming timeline progress equals physical arrival.

## Animation catalogue

V2 uses the 47 built-in motions below ([V1 motions](../motion/motions/v1/) live separately).

| Motion | Intent | File |
| --- | --- | --- |
| `arrival_notice` | Raise and turn the head first, then rise into the attentive posture and return. | [V2](../motion/motions/v2/arrival_notice.yaml) |
| `attention_left` | A small grounded turn toward the confirmed speaker on the left. | [V2](../motion/motions/v2/attention_left.yaml) |
| `attention_right` | A small grounded turn toward the confirmed speaker on the right. | [V2](../motion/motions/v2/attention_right.yaml) |
| `delight_lift` | Lift the head and look up into the delight posture. | [V2](../motion/motions/v2/delight_lift.yaml) |
| `farewell_watch` | Watch the place where the person left, sigh gently, and return. | [V2](../motion/motions/v2/farewell_watch.yaml) |
| `idle_attentive_hold` | Raise the gaze and lift the arm slowly while holding an attentive posture. | [V2](../motion/motions/v2/idle_attentive_hold.yaml) |
| `idle_breathe` | Raise the head slowly, lift the arm with it, and settle back. | [V2](../motion/motions/v2/idle_breathe.yaml) |
| `idle_directional_hold` | Raise the gaze and lift the arm slowly while holding a directional or desk anchor. | [V2](../motion/motions/v2/idle_directional_hold.yaml) |
| `idle_head_curiosity` | Raise the gaze and turn the neck slowly to inspect the surroundings, with a small arm lift. | [V2](../motion/motions/v2/idle_head_curiosity.yaml) |
| `idle_micro_glance` | Turn the base slowly for a brief glance while the arm eases back, with a full return. | [V2](../motion/motions/v2/idle_micro_glance.yaml) |
| `idle_shoulder_adjust` | Redistribute the shoulder and elbow slowly, then return. | [V2](../motion/motions/v2/idle_shoulder_adjust.yaml) |
| `idle_soft_head_shake` | Turn the neck slowly left and right while the arm eases back, with a soft return. | [V2](../motion/motions/v2/idle_soft_head_shake.yaml) |
| `idle_weight_shift` | Shift the shoulder and elbow slowly, then restore their balance. | [V2](../motion/motions/v2/idle_weight_shift.yaml) |
| `listen_lean_in` | Pull back a little, lean toward the speaker, hold, and return. | [V2](../motion/motions/v2/listen_lean_in.yaml) |
| `look_around` | Deliberate three-stop desk search, neck leading the base, then returning to its anchor. | [V2](../motion/motions/v2/look_around.yaml) |
| `look_at_left` | Turn directly toward the predefined target on Orion's left. | [V2](../motion/motions/v2/look_at_left.yaml) |
| `look_at_left_expressive` | Notice, lean toward, and settle on the predefined left target. | [V2](../motion/motions/v2/look_at_left_expressive.yaml) |
| `look_at_right` | Turn directly toward the predefined target on Orion's right. | [V2](../motion/motions/v2/look_at_right.yaml) |
| `look_at_right_expressive` | Notice, lean toward, and settle on the predefined right target. | [V2](../motion/motions/v2/look_at_right_expressive.yaml) |
| `mutual_gaze_away` | Turn the neck aside with a raised gaze and a small arm lift, then return. | [V2](../motion/motions/v2/mutual_gaze_away.yaml) |
| `mutual_gaze_hi` | Raise the gaze and lift the arm in a clear greeting, then return. | [V2](../motion/motions/v2/mutual_gaze_hi.yaml) |
| `mutual_gaze_notice` | Raise the gaze, turn the neck with curiosity while the arm lifts, and return. | [V2](../motion/motions/v2/mutual_gaze_notice.yaml) |
| `mutual_gaze_notice_demo` | Meet the look with a clear nod, turn the neck with curiosity, and return. | [V2](../motion/motions/v2/mutual_gaze_notice_demo.yaml) |
| `peek_new_object` | Arm-led desk peek with a curious neck turn and a full anchor return. | [V2](../motion/motions/v2/peek_new_object.yaml) |
| `pet_notice` | Lift into delight, follow the pet briefly, and return to the held anchor. | [V2](../motion/motions/v2/pet_notice.yaml) |
| `return_home` | Move from the current joint state to Orion's home pose. | [V2](../motion/motions/v2/return_home.yaml) |
| `sleep_settle` | Lift a little, then sink the head and arm slowly, and return. | [V2](../motion/motions/v2/sleep_settle.yaml) |
| `speak_calm_sway` | Sway the neck and arm with alternating raised and lowered gaze. | [V2](../motion/motions/v2/speak_calm_sway.yaml) |
| `speak_emphasis_nod` | Nod down at a phrase boundary, lift back, and settle at the anchor. | [V2](../motion/motions/v2/speak_emphasis_nod.yaml) |
| `speak_explanatory_lean` | Lean forward with a raised gaze for explanatory emphasis, then return. | [V2](../motion/motions/v2/speak_explanatory_lean.yaml) |
| `speak_react_agree` | Make two clear downward nods with the arm dipping along. | [V2](../motion/motions/v2/speak_react_agree.yaml) |
| `speak_react_curious` | Raise the gaze and turn the neck with a slight upright lean. | [V2](../motion/motions/v2/speak_react_curious.yaml) |
| `speak_react_disagree` | A clear three-beat head shake led by the neck, with the base following. | [V2](../motion/motions/v2/speak_react_disagree.yaml) |
| `speak_react_happy` | Lift the head and gaze with two bright upward bounces. | [V2](../motion/motions/v2/speak_react_happy.yaml) |
| `speak_react_laugh` | Make three raised head bounces with the arm bouncing along. | [V2](../motion/motions/v2/speak_react_laugh.yaml) |
| `speak_react_shy` | Turn away and lower the head, peek back, then return. | [V2](../motion/motions/v2/speak_react_shy.yaml) |
| `speak_react_sorry` | Shrink back and lower the head after a mistake. | [V2](../motion/motions/v2/speak_react_sorry.yaml) |
| `speak_react_surprised` | Pull the arm back and raise the head for a surprised pause. | [V2](../motion/motions/v2/speak_react_surprised.yaml) |
| `speak_react_sympathy` | Sink with the head down, the arm lowered and a soft turn, hold, and rise. | [V2](../motion/motions/v2/speak_react_sympathy.yaml) |
| `speak_react_thinking` | Look aside and slightly up during a thoughtful pause. | [V2](../motion/motions/v2/speak_react_thinking.yaml) |
| `speak_react_unsure` | Look slightly down while turning the neck from side to side, with the arm easing back. | [V2](../motion/motions/v2/speak_react_unsure.yaml) |
| `speak_reflective_tilt` | Turn the neck aside, lower the gaze and ease back, then return quietly. | [V2](../motion/motions/v2/speak_reflective_tilt.yaml) |
| `think_beat_a` | Glance up and aside to think, hold, and return. | [V2](../motion/motions/v2/think_beat_a.yaml) |
| `think_beat_b` | Lower the head and sink a little in thought, hold, and return. | [V2](../motion/motions/v2/think_beat_b.yaml) |
| `wake_stretch` | Dip, stretch tall with the head up, shake out, and settle. | [V2](../motion/motions/v2/wake_stretch.yaml) |
| `wave_back` | Turn the head to the person, lift the arm, sway it side to side as a wave, nod, and return. | [V2](../motion/motions/v2/wave_back.yaml) |
| `yawn` | Dip, rise tall with the head up, hold, then sink back. | [V2](../motion/motions/v2/yawn.yaml) |

V2 poses live in [poses.yaml](../motion/config/v2/poses.yaml) and cover `home`, `rest`, `attentive`, `delight`, desk-work/reach postures, and look targets (including anticipation, lean and overshoot keyframes). Note that positive V2 `head_pitch_joint` lowers the gaze, while positive `head_roll_joint` swivels the head right (there is no sideways head tilt on V2).

The [intent table](../motion/config/v2/movement-intents.yaml) defines expected movement directions, main keyframes, movement sizes and preview contexts for automated MuJoCo validation: operating targets retain a 3° joint-limit margin, and look targets keep combined base yaw and neck swivel within 90°. See [preview all V2 movements](../simulation/mujoco/README.md#preview-all-v2-movements) for full preview options.

1. Compile the catalogue and validate all movement checks without rendering videos.

   ```bash
   .venv/bin/python scripts/preview_movements.py --validate-only --output ../scratch/motion-checks
   ```

2. Render preview videos and the review index when inspecting changes visually.

   ```bash
   .venv/bin/python scripts/preview_movements.py
   ```
