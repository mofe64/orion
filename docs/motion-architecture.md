# How Orion moves

Orion is a lamp with five servo joints. A request such as "acknowledge the
person on the left" becomes a short sequence of named poses. The Rust runtime
(`oriond`) turns that sequence into one smooth trajectory and sends joint goals
to the servos 50 times a second. Exact schemas, constants and the animation
catalogue are in the [motion reference](motion-reference.md).

## Key terms

| Term | Meaning |
| --- | --- |
| Joint | One of five servos: `base_yaw_joint`, `shoulder_pitch_joint`, `elbow_pitch_joint`, `head_roll_joint`, `head_pitch_joint`. Angles are radians. |
| Pose | A named target for all five joints, such as `home` or `look_left`. |
| Motion | An ordered list of keyframes. Each keyframe names a pose (absolute motion) or offsets from an anchor (anchor-relative motion). |
| Anchor | A complete measured pose that idle and speech animation move around and always return to. Animation never moves the anchor itself. |
| Arrival | How a keyframe is reached. `through` keeps moving through it; `settle` stops there with zero velocity. |
| Style | Named timing character (tempo, smoothness, amplitude). Styles never change safety limits. |
| Scene | A motion plus light and sound, run on one clock and synchronised by motion markers. |
| Settling | After a motion's timeline ends, the runtime waits until measured feedback is still and on target before reporting `completed`. |

## From request to servo

```text
User, voice or Studio request
        │  names a pose, motion, scene or character state
        ▼
Character or scene coordinator      decides what may run now
        ▼
Motion definition                   absolute poses, or offsets around an anchor
        ▼
MotionSequence                      resolves targets, one uniform scale, markers
        ▼
CompiledTrajectory                  quintic position/velocity/acceleration curves
        ▼
RuntimeCore, every 20 ms            read feedback → sample → write all five goals
        ▼
Driver                              calibration check, radians ↔ servo encoder
        ▼
STS3215 servos   or   MuJoCo simulator
```

1. **A request names a capability.** Studio talks to the authenticated gateway,
   which sends a command over the Pi's local Unix socket. Commands name a pose,
   motion, scene or character state. They cannot contain raw servo writes or
   arbitrary joint streams.
2. **The runtime picks a start state.** A new movement starts from the latest
   measured position and velocity. Speech continuations instead start from the
   commanded position, velocity and acceleration, so each extension does not
   restart from lagging feedback.
3. **Targets are resolved.** Absolute keyframes use their named pose.
   Anchor-relative keyframes add their offsets to the immutable anchor. If the
   offsets would leave the calibrated range, the whole clip is scaled down by
   one factor, which keeps its shape.
4. **The whole action is compiled at once.** The compiler sees every remaining
   keyframe, so velocity carries smoothly through `through` keyframes instead of
   easing to a stop at each one. It then removes unrequested overshoot, slows
   only segments that would exceed the servo's speed, and checks every 20 ms
   sample against calibration.
5. **`RuntimeCore` executes at 50 Hz.** Each cycle reads all joint feedback,
   samples the trajectory, writes one synchronised goal packet for all five
   joints, and publishes status.
6. **Measured feedback decides completion.** When the timeline ends the run
   moves to `settling`. It becomes `completed` only after every joint stays
   within position and velocity tolerance for the settle window, or
   `timed_out` if it never does. `stop` produces `cancelled`.

The hardware and MuJoCo backends implement the same `RuntimeDriver` interface
and receive the same joint samples. MuJoCo is a physics backend, not a second
animation system.

## Why the movement looks fluid

- Authors use a few readable poses rather than dense motor samples.
- Internal keyframes use `through` unless a stop is part of the acting.
- The compiler shares velocity and acceleration across those keyframes.
- Joints get slightly different path character, so they do not move in lockstep.
- Coordinated joints trace curved arcs with the head.
- Speech is one continuous motion run that extends as audio arrives and
  settles once at the end.
- Supporting joints follow the main action instead of oscillating on their own.

A joint can briefly reach zero velocity when it reverses direction. The
compiler never inserts a held stop unless the keyframe says `settle`.

## Character mode

`oriond` starts character mode after it initialises, on V1 and V2.
`--character-on-start off` is the maintenance override. Startup configures the
servos, enables holding torque, moves to `home` and captures the anchor. Only a
measured arrival at home enters idle; a cancelled or timed-out start leaves
character mode off.

```text
Off ── character start ──▶ Starting ── home completed ──▶ HomeIdle
                                                             │ held elsewhere
                                                             ▼
                                                          PoseIdle
                                          ┌─────────────────┼─────────────────┐
                                          ▼                 ▼                 ▼
                                      Listening         Thinking          Speaking
                                          └─────────────────┴───────┬─────────┘
                                                                    ▼
                                                           ForegroundScene
                                                                    ▼
                                                     Settling ──▶ HomeIdle / PoseIdle

Any enabled state ── character stop ──▶ ShuttingDown ──▶ Off
```

Stopping character mode cancels owned scene and speech work, returns home,
clears character lighting and leaves holding torque on. Moving to the
mechanical `rest` pose and releasing torque are separate steps, sequenced by
the [automatic rest coordinator](system-architecture.md#automatic-rest-and-waking).

### Who wins when behaviours compete

1. Shutdown, cancellation and torque release.
2. An explicit foreground scene or motion.
3. Speech.
4. Listening or thinking reactions.
5. Autonomous idle.
6. Background idle lighting.

The scene coordinator owns scene tracks, the speech coordinator owns audio
playback, and the character coordinator owns the anchor, idle schedule and
generated speech motion. `RuntimeCore` is the only thing that executes
movement for all three.

### Anchors

The anchor is captured when Orion enters an idle context. Every idle and speech
clip is resolved from it:

```text
target[joint] = anchor[joint] + style_amplitude × uniform_scale × offset[joint]
```

Each clip ends with a zero-offset `settle`, so it returns exactly to the
anchor. Offsets never accumulate, and recovery after an interruption is
predictable. A completed foreground scene, or a direct foreground motion that
ends holding, sets the next anchor from the measured pose. Speech, reactions,
idle, and failed or cancelled scenes never change it.

### Idle

Two independent randomised timers pick the next idle: a micro idle every 8–20
seconds and a larger idle every 35–75 seconds. The anchor pose's
`idle_profile` chooses suitable clips; directional poses avoid yaw clips that
would push into a range limit. Each play varies direction, amplitude and tempo
slightly from a seeded random generator, never repeats the previous clip, and
plays no sound. Background light comes from the anchor pose's
`default_lighting`.

### Thinking and speech

Repeated thinking requests during one voice turn keep the current thinking
movement instead of restarting it. When speech takes over, motion continues
from the commanded thinking state.

Speech animation is driven by the reply audio. The runtime measures loudness in
20 ms frames, finds quiet gaps and phrase peaks, and plans one head-led
performance:

- every phrase leads with the head, and the shoulder and elbow follow later;
- emphasis nods land just before loud peaks;
- a larger body beat appears only on strong, well-spaced peaks;
- quiet gaps can hold the current phrase pose;
- the performance ends with one settle at the anchor.

Long or streamed replies are planned in pieces and extend the same motion run
as audio arrives. Speech motion is best effort: if it fails to plan, the audio
still plays. If audio fails, the motion settles back to the anchor. The exact
values are in the [speech performance policy](motion-reference.md#speech-performance-policy).

### Voice attention

Confirmed voice sessions can request small absolute turns toward the speaker
(`attention_left` and `attention_right`). The
[runtime attention contract](../runtime/README.md#character-startup-and-voice-attention)
covers their anchor, return timing and interruption.

## Failure containment

- A speech motion failure does not silence speech.
- An audio failure cancels speech animation and settles to the anchor.
- An idle timeout is diagnostic and does not turn character mode off.
- A scene movement timeout ends the scene and stops its audio.
- Invalid assets fail loading or reload before anything moves.
- Targets outside calibration are rejected before any servo command.
- Activation copies present positions into the goal registers before enabling
  torque, so the arm does not jump.
- A few missed servo replies are tolerated; sustained bus failure cancels the
  movement and reports `bus_fault` while the daemon keeps serving.

## Designing animation

Orion should feel calm, curious, warm and attentive: alive enough to read
intent, restrained enough not to compete with the conversation. The governing
rule is one primary idea at a time. Six qualities guide every design:

- **Ease:** begin, travel and settle without harshness.
- **Legibility:** the target, state or acknowledgement is easy to read.
- **Economy:** use the smallest movement, light or sound that works.
- **Grounding:** expression follows real runtime state, not decorative randomness.
- **Natural timing:** pauses and overlap feel intentional but remain testable.
- **Temperament:** calm and competent rather than hyperactive.

The classic animation principles map onto Orion like this:

| Principle | How Orion applies it |
| --- | --- |
| Squash and stretch | Shoulder and elbow close the silhouette before an opening or lift |
| Anticipation | Expressive turns start with a small opposite yaw |
| Staging | One joint group leads; light and sound land on the same beat |
| Pose to pose | Named poses for actions; idle and speech vary approved shapes |
| Follow-through and overlap | Supporting joints arrive after the lead; speech delays the body after the head |
| Slow in and slow out | The compiler shares derivatives at `through` and stops only at `settle` |
| Arcs | Joints are reviewed together in MuJoCo and on hardware |
| Secondary action | Small elbow follow, counter-tilt or light supports the main idea |
| Timing | Styles change tempo, smoothness, amplitude and settle weight |
| Exaggeration | Authored overshoot and emphasis nods stay within calibration |
| Solid drawing | Every held pose is checked from useful angles under gravity |
| Appeal | Asymmetry, warm cues, a forward eyeline and purposeful stillness |

Before adding an asset, answer:

1. What is the single idea and which joints lead it?
2. Which silhouette must read at the main drawing?
3. What anticipation announces it?
4. Which joints follow, and how do they stay secondary?
5. Where is a real `settle` justified?
6. Does it trace a coordinated arc?
7. How does timing express weight?
8. Which anchors must support it?
9. What happens on interruption, timeout or cancellation?
10. What MuJoCo and physical checks show the acting reads?

Add the asset to the [animation catalogue](motion-reference.md#animation-catalogue)
in the same change.

## How it is validated

- **Schema checks** reject incomplete or ambiguous poses, motions and scenes.
- **Compiler tests** check continuity, exact targets, speed retiming,
  calibration containment and interruption.
- **Catalogue tests** compile every built-in pose and motion against calibration.
- **Native MuJoCo tests** run the real daemon against the physics backend.
- **Physical review** covers what joint-space tests cannot: silhouette, timing,
  cable clearance, sound, light and appeal.

## Where the code lives

| Concern | Source |
| --- | --- |
| Poses | `runtime/src/motion/pose.rs` |
| Motions, relative resolution, amplitude scaling | `runtime/src/motion/library.rs` |
| Styles | `runtime/src/motion/style.rs` |
| Trajectory compiler | `runtime/src/motion/trajectory.rs` |
| Movement lifecycle and 50 Hz sampling | `runtime/src/control/core.rs` |
| Character state, idle, speech performance | `runtime/src/expression/character.rs` |
| Speech audio and energy analysis | `runtime/src/expression/speech.rs` |
| Scenes and markers | `runtime/src/expression/scene.rs` |
| Daemon loop and command priority | `runtime/src/app/server.rs`, `runtime/src/app/commands.rs` |
| Calibration and radians conversion | `runtime/src/motion/calibration.rs`, `runtime/src/devices/sts3215/driver.rs` |
| Servo packets | `runtime/src/devices/sts3215/transport.rs` |
