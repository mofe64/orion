# Author and validate Orion motion

Install a working Rust toolchain and the repository Python environment before
changing movement. Physical validation also requires a commissioned Orion and
an operator with access to the hardware power or torque interruption.

Follow the [character animation design](../explanation/character-animation.md),
[motion asset reference](../reference/motion-assets.md), and
[trajectory reference](../reference/trajectory-and-joint-control.md) throughout
the change. The amount of artistic review may vary, but schema validation,
calibrated compilation, simulator validation, and documentation updates do not.

## 1. Write the behavior brief

Before editing joint values, record:

- the user-facing purpose;
- the primary action and leading joint group;
- the intended start and final anchor;
- anticipation, dominant drawing, follow-through, and settle;
- whether the asset is absolute or anchor-relative;
- light or sound markers required by a scene;
- interruption and cancellation behavior; and
- the visual evidence that will constitute acceptance.

If the action cannot be summarized as one primary idea, split it into separate
motions or scenes. Do not solve an unclear behavior by adding more keyframes.

## 2. Choose the correct asset type

Use a **pose** for a complete readable silhouette. Use an **absolute motion**
when named poses and a specific final orientation are the action. Use an
**anchor-relative motion** for reusable idle or speaking detail that must work
around several powered anchors.

Use a **scene** only when motion, light, and audio must be coordinated. A scene
still references named motion; it does not contain joint targets.

Generated speech sequencing belongs in `CharacterCoordinator`, but its source
drawings remain ordinary reviewed anchor-relative motion assets.

## 3. Author or tune poses

Edit complete poses through the calibrated MuJoCo pose editor:

```bash
../mujoco-local/.venv/bin/python simulation/mujoco/pose_editor.py --check
../mujoco-local/.venv/bin/python simulation/mujoco/pose_editor.py
```

Review each pose for:

- a clear silhouette from the expected human viewpoint;
- a useful forward eyeline;
- gravity-supported elbow and head posture;
- room for the intended anticipation and follow-through;
- no collision, floor contact, cable-hostile fold, or calibration-edge pose;
- correct role metadata (`idle_anchor`, `transition`, or shutdown-only); and
- a default light that supports rather than obscures the pose.

Do not edit calibration to make an authored pose pass. Calibration describes
the accepted physical mechanism; the pose must fit it.

## 4. Author the motion drawings

Create or update one YAML file under the correct `motion/motions/` category.
Follow the exact [motion schema](../reference/motion-assets.md#motion-schema).

Use the fewest drawings that communicate the action:

1. optional anticipation;
2. dominant action or committed lean;
3. optional authored overshoot or counter-shape;
4. final settle.

Choose `through` for internal drawings unless visible stillness is part of the
acting. Use `settle` only where the complete character should intentionally
arrive at rest. A direction reversal does not need a settle.

Place markers on semantic beats such as `notice`, `emphasis`, or `settled`, not
on guessed wall-clock times. Scene events attached to markers remain aligned
after speed retiming.

## 5. Choose a style before changing durations

Select the style whose character matches the action. First adjust authored
pose relationships and segment timing; change the global style table only when
several motions need the same additional timing vocabulary.

A style change affects every asset using it and therefore requires catalog-wide
validation. Never add calibration ranges or motor limits to a style.

## 6. Compile the exact trajectory

Build the Rust exporter:

```bash
cargo build --manifest-path runtime/Cargo.toml --bin orion-trajectory
```

Compile an absolute motion from a representative start pose:

```bash
runtime/target/debug/orion-trajectory \
  --motion look_at_left_expressive \
  --start-pose attentive \
  --pose-file motion/config/poses.yaml \
  --motions-directory motion/motions \
  --calibration simulation/mujoco/config/servo_calibration.json
```

For an anchor-relative clip, also name the anchor:

```bash
runtime/target/debug/orion-trajectory \
  --motion idle_breathe \
  --start-pose home \
  --anchor-pose home \
  --pose-file motion/config/poses.yaml \
  --motions-directory motion/motions \
  --calibration simulation/mujoco/config/servo_calibration.json
```

Inspect the emitted document for:

- `control_rate_hz: 50`;
- the expected amplitude scale;
- peak velocity below the hardware profile ceiling;
- exact marker order and arrival times;
- exact final pose or anchor;
- no unintended stopped samples around internal `through` drawings; and
- calibration ranges for all five joints.

Do not hand-author preview samples. Studio, Python diagnostics, and MuJoCo must
consume the Rust compiler output.

## 7. Run automated validation

Run the complete runtime suite:

```bash
cargo fmt --check --manifest-path runtime/Cargo.toml
cargo test --manifest-path runtime/Cargo.toml --all-targets
```

Run Python consumer and reporting tests:

```bash
PYTHONPATH=motion .venv/bin/python -m pytest -q motion/test
```

Run the MuJoCo suite when pose geometry, trajectory behavior, calibration,
stability policy, or the backend changes:

```bash
.venv/bin/python -m pytest -q simulation/mujoco
```

Targeted tests are useful during iteration, but the final gate must cover the
whole catalog because poses and styles have shared consumers.

### Required test properties

Add or update tests when behavior changes. Evidence should cover the relevant
properties:

- schema and unknown-field rejection;
- exact semantic target and marker preservation;
- position, velocity, and acceleration continuity at `through` boundaries;
- zero velocity and acceleration at `settle`;
- no extra polynomial overshoot;
- speed retiming below the STS3215 ceiling;
- calibrated interruption from measured position and velocity;
- uniform relative scaling and exact anchor return;
- deterministic seeded idle or speech selection;
- correct priority, cancellation, and terminal lifecycle; and
- shared hardware/MuJoCo execution behavior.

Tests should assert the invariant, not a private implementation detail. For
animation hierarchy, however, numeric constraints such as “ordinary body
action remains smaller than the head lead” are appropriate because they
protect an intentional character rule.

## 8. Review in MuJoCo

Run the real daemon against the MuJoCo driver:

```bash
runtime/target/debug/oriond --serve --backend mujoco --start-pose home
```

From a second terminal:

```bash
runtime/target/debug/oriond --configure
runtime/target/debug/oriond --enable
runtime/target/debug/oriond --goto home --duration 3.0 --wait
runtime/target/debug/oriond --play MOTION_NAME --wait
runtime/target/debug/oriond --status
```

Review at normal speed before using slow motion. Check:

- the primary idea is readable without knowing the keyframe names;
- anticipation is smaller and opposite where appropriate;
- the dominant drawing has a clear silhouette;
- supporting joints overlap without becoming a second action;
- internal drawings do not create a perceptible stop;
- the path reads as a coordinated arc;
- the final settle has the intended weight; and
- collision, contact, base motion, and torque diagnostics remain acceptable.

Slow-motion inspection is for diagnosing a problem, not judging overall
timing.

## 9. Update the animation review and technical references

For a built-in asset, update
[`animation-principles-review.md`](../reference/animation-principles-review.md)
with its primary action, anticipation/follow-through, timing, secondary action,
and intended silhouette.

Update the maintained reference for each changed behavior:

| Change | Reference to update |
| --- | --- |
| Pose or motion fields and invariants | `docs/reference/motion-assets.md` |
| Compiler, retiming, calibration, runtime, or servo behavior | `docs/reference/trajectory-and-joint-control.md` |
| Character priority, idle, speech, or animation language | `docs/explanation/character-animation.md` |
| Cross-component control flow | `docs/explanation/motion-and-animation-architecture.md` |
| Commands and deployment operations | `runtime/README.md` |
| Physical acceptance gates | `docs/how-to/validate-character-v2.md` |

Maintain one technical description for each behavior and link to it from any
affected component README.

## 10. Run supervised physical acceptance

Run the motion on physical Orion when perceived timing, gravity load, sound,
lighting, cable clearance, or joint prominence changes. Follow
[Validate Orion character on physical hardware](validate-character-v2.md).

At minimum:

1. record the deployed Git revision and calibration hash;
2. begin with torque off and a clear movement envelope;
3. start character mode and verify the powered home anchor;
4. run the changed action in its real interaction context;
5. observe diagnostics and retain the semantic run ID;
6. test interruption when interruption behavior changed;
7. stop character mode;
8. move to mechanical `rest` and wait for measured completion; and
9. disable torque only after rest is confirmed.

Stop immediately for contact, cable tension, harsh servo noise, unexpected
direction, loss of control, or movement toward a mechanical limit.

## Review checklist

Before merging, verify:

- [ ] The behavior brief names one primary idea.
- [ ] Asset type and motion space are correct.
- [ ] Every internal arrival has deliberate `through` or `settle` intent.
- [ ] Markers name semantic beats.
- [ ] All five-joint targets fit calibration.
- [ ] Relative clips preserve shape under uniform scaling and return exactly.
- [ ] Rust formatting and complete tests pass.
- [ ] Python consumers and relevant MuJoCo tests pass.
- [ ] The animation-principles review reflects the changed catalog.
- [ ] Record supervised physical acceptance when required.
- [ ] Return the robot to mechanical rest and disable torque after testing.
