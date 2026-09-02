# Motion and animation architecture

| Document contract | Value |
| --- | --- |
| Status | Current architecture |
| Audience | Motion designers, runtime engineers, Studio engineers, and reviewers |
| Owns | The end-to-end mental model, component boundaries, and control flow |
| Defers to | [Motion asset reference](../reference/motion-assets.md) for schemas and [trajectory and joint-control reference](../reference/trajectory-and-joint-control.md) for exact algorithms |
| Code authority | `runtime/src/{character,motion,trajectory,daemon,driver,transport}.rs` |

Orion separates **what the character intends to do** from **how the motors are
commanded**. A semantic action such as “acknowledge the person on the left” is
selected at the character or scene layer. The Rust runtime resolves that action
into joint-space waypoints, compiles the whole action into one continuous
trajectory, and samples it at 50 Hz. Only the hardware driver knows how radians
map to STS3215 encoder values.

That separation is the central architectural rule. Character code never writes
servo registers, and the servo layer never decides which expression Orion
should perform.

## The motion stack

```text
User or voice event
        │
        ▼
Semantic operation
character state / named motion / named scene
        │
        ▼
Expression coordination
CharacterCoordinator or SceneCoordinator
        │
        ▼
Motion definition
absolute poses or offsets around an immutable anchor
        │
        ▼
MotionSequence
resolve targets + uniform relative scale + markers
        │
        ▼
CompiledTrajectory
piecewise quintic position/velocity/acceleration functions
        │
        ▼
RuntimeCore at 50 Hz
read feedback → sample command → synchronized write → publish state
        │
        ▼
RuntimeDriver
calibration validation + radians/encoder conversion
        │
        ▼
Sts3215Transport                         MuJoCoDriver
synchronized serial read/write     or   simulated joint interface
```

Each boundary has one responsibility:

- **Poses** define complete, readable silhouettes.
- **Motions** define ordered dramatic drawings, timing, arrival intent, and
  markers.
- **Styles** describe artistic timing and derivative character; they do not
  contain safety limits.
- **Scenes** coordinate motion, light, and sound under one monotonic clock.
- **Character coordination** decides when idle, reaction, speech, or foreground
  behavior may run.
- **Trajectory compilation** turns the entire action into continuous joint
  functions.
- **Runtime lifecycle** owns execution, cancellation, measured settling, and
  status.
- **The driver and transport** own calibrated hardware conversion and the
  servo bus.

## Semantic assets

### Poses are complete silhouettes

A pose supplies a target in radians for all five Orion joints. It may also
carry tags, an idle profile, and a default light effect. Poses serve three
different roles:

- powered anchors such as `home`, `attentive`, and `look_left`;
- transition drawings such as anticipation, lean, and authored overshoot;
- the shutdown-only mechanical `rest` pose.

Transition drawings are not states that the character holds. They are points
through which a larger action flows. Powered anchors may become the immutable
reference for relative idle and speaking animation. Mechanical `rest` is never
an animation anchor.

### Motions define arrival intent

A motion is either:

- **absolute**, where each keyframe references a complete named pose; or
- **anchor-relative**, where each keyframe supplies offsets from a separately
  captured anchor.

Every keyframe declares one of two arrival modes:

- `through` means the action should carry motion through the drawing;
- `settle` means velocity and acceleration deliberately reach zero.

This is more than file syntax. Arrival intent determines the boundary
conditions passed to the trajectory compiler. An internal `through` drawing is
not compiled as a stop followed by a new move. A `settle` drawing is a real
stop and is the only kind that may hold.

### Styles are artistic policy

The named style supplies tempo, tangent tension, joint-lag character,
amplitude, overshoot character, and settle character. These values change how
the path reads without changing the authored semantic targets or calibrated
position limits.

For example, `living_idle` lowers amplitude and tangent energy, while
`expressive_turn` preserves a stronger authored arc. `return_home` is slower
and more heavily settled. The exact style table belongs to the
[motion asset reference](../reference/motion-assets.md#motion-styles).

## End-to-end control flow

### 1. A semantic request enters `oriond`

Studio communicates with the authenticated gateway, and the gateway sends a
narrow command over the Pi-local Unix socket. Local tools use the same socket.
Commands name capabilities such as a pose, motion, scene, or character state;
they cannot contain raw register writes or arbitrary joint streams.

The command handler enforces ownership and priority before starting motion. A
foreground scene cancels lower-priority speech and preempts an autonomous idle.
Character shutdown cancels scene and speech work before returning home.

### 2. Targets are resolved from measured state

`RuntimeCore` uses the most recent measured position and velocity as the start
of a new trajectory. It does not assume that the preceding command reached an
ideal pose.

For an absolute motion, keyframes already contain complete pose targets. For a
character-owned relative motion, every offset is resolved from the immutable
idle or speech anchor even when the interruption begins elsewhere. This gives
two simultaneous guarantees:

- the blend starts from physical reality; and
- repeated relative animation cannot move its reference point or accumulate
  drift.

Before compilation, relative motion is uniformly scaled if the full offsets
would exceed live calibrated ranges. Scaling the entire clip preserves its
shape. Individual joints are not independently clipped into a distorted pose.

### 3. The complete action is compiled once

`MotionSequence` converts resolved keyframes to trajectory waypoints and calls
the Rust `CompiledTrajectory`. The compiler sees the start state and every
future drawing at the same time. It can therefore derive compatible internal
velocities and accelerations instead of restarting an easing curve at each
keyframe.

Each joint and each segment receives a quintic polynomial constrained by
position, velocity, and acceleration at both ends. Neighboring segments share
the same derivative values at a `through` waypoint, which gives continuous
position, velocity, and acceleration across the join.

The compiler then:

1. attenuates local derivatives that would create unrequested overshoot;
2. measures segment peak velocities;
3. stretches only segments that exceed the STS3215 capability ceiling; and
4. checks calibrated interruption samples at the same 50 Hz rate used by the
   runtime.

Authored overshoot remains a real keyframe. “Unrequested overshoot” means a
polynomial leaving the interval between its two authored endpoints.

### 4. `RuntimeCore` executes at 50 Hz

The outer `oriond` loop uses a monotonic clock and a 20 ms period. On each
cycle, `RuntimeCore`:

1. synchronously reads all joint feedback;
2. computes elapsed motion time;
3. samples the compiled trajectory at that time;
4. sends one synchronized five-joint goal write;
5. updates keyframe, marker, progress, and lifecycle metadata; and
6. publishes the sampled feedback snapshot.

Reading before writing preserves a consistent snapshot of the state that
preceded the cycle's new command. Scene, speech, lighting, character, and Unix
command processing then run around the same monotonic loop.

Hardware and MuJoCo implement the same `RuntimeDriver` interface. They receive
the same joint-space samples and run under the same daemon lifecycle. MuJoCo
is therefore a physics backend, not a second animation implementation.

### 5. Completion is measured, not assumed

Finishing the authored duration moves a run from `executing` to `settling`.
The runtime continues reading feedback and requires every joint to remain
inside both the position and velocity tolerances for the configured settle
window. It reports `completed` only after that measured condition holds.

A run becomes `timed_out` if feedback does not settle before the timeout.
Cancellation is explicit and produces `cancelled`. The daemon keeps the active
run and most recent terminal run for diagnostics; it is not a durable event
store.

## Why the movement is fluid

Fluidity is the result of several layers working together:

- Motion authors use a small number of readable poses rather than dense motor
  samples.
- Internal drawings use `through` unless a stop is part of the acting.
- The compiler carries compatible derivatives across those drawings.
- Joint-specific derivative character prevents every axis from sharing an
  identical path shape.
- Coordinated joint targets create visual arcs in the head and lamp body.
- Interruption begins from measured position and velocity.
- Speech is compiled as one utterance-length performance rather than a queue of
  independently settled gestures.
- Secondary joints follow the primary action instead of moving as an unrelated
  periodic oscillator.

Continuous derivatives do not mean that every joint is always moving. A joint
may instantaneously reach zero velocity at a direction reversal. The important
distinction is that the compiler does not insert a held zero-velocity plateau
unless the authored arrival is `settle`.

## Character and scene ownership

The effective behavior priority is:

1. shutdown, cancellation, and release;
2. explicit foreground scene or motion;
3. speech;
4. listening and thinking reaction state;
5. autonomous idle;
6. background idle lighting.

The scene coordinator owns parallel scene tracks until they are terminal. The
speech coordinator owns WAV lifecycle and playback. The character coordinator
owns the animation anchor, state transitions, randomized idle schedule, and
generated speaking motion. `RuntimeCore` remains the only movement executor
for all three.

Completing a foreground scene may establish its final measured pose as the
next idle anchor. A direct foreground motion captures the measured pose it
leaves holding when the run terminates. Speech and idle always return to the
anchor they inherited; they never replace it. Failed or cancelled scenes do
not establish a new anchor.

## Failure containment

The layers fail independently where product behavior requires it:

- A generated speech motion failure does not stop audible speech.
- An audio failure cancels speech animation and settles back to the anchor.
- A low-priority idle timeout remains diagnostic and does not turn character
  mode off.
- A scene movement timeout makes the scene terminal and stops its owned audio.
- Invalid assets fail catalog loading or transactional reload before execution.
- A target outside calibration is rejected before any joint command is sent.
- Driver activation seeds goal registers from present encoder positions before
  enabling torque, preventing an activation jump.

## Validation model

Orion validates the system at multiple boundaries:

- **Schema validation** proves that pose, motion, and scene intent is complete
  and unambiguous.
- **Compilation tests** prove derivative continuity, exact authored targets,
  speed retiming, calibration containment, and interruption behavior.
- **Catalog tests** compile every built-in pose and motion against the tracked
  calibration.
- **Native MuJoCo tests** run the real daemon state machine against the physics
  backend.
- **Physical acceptance** proves qualities that joint-space tests cannot:
  silhouette, perceived prominence, cable clearance, sound level, lighting,
  and character appeal.

Use [Author and validate motion](../how-to/author-and-validate-motion.md) for the
change workflow and [Validate the character on physical Orion](../how-to/validate-character-v2.md)
for the supervised hardware gates.

## Source map

| Concern | Authoritative source |
| --- | --- |
| Pose parsing and semantic metadata | `runtime/src/pose.rs` |
| Motion parsing, relative resolution, and amplitude scaling | `runtime/src/motion.rs` |
| Motion styles | `runtime/src/style.rs` |
| Quintic compilation, derivatives, overshoot control, and retiming | `runtime/src/trajectory.rs` |
| Movement lifecycle and 50 Hz sampling | `runtime/src/daemon.rs` |
| Character state, idle, and generated speech performance | `runtime/src/character.rs` |
| WAV lifecycle and energy analysis | `runtime/src/speech.rs` |
| Scene tracks and marker dispatch | `runtime/src/scene.rs` |
| Daemon loop and command priority | `runtime/src/main.rs` |
| Calibration and radians/raw conversion | `runtime/src/calibration.rs`, `runtime/src/driver.rs` |
| Synchronized STS3215 packets | `runtime/src/transport.rs` |
