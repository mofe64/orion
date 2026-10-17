# How Orion moves

Orion turns named actions into joint goals at 50 Hz. The Rust runtime controls execution for hardware and MuJoCo. Other processes request actions through checked interfaces.

## Key terms

| Term | Meaning |
| --- | --- |
| Pose | A named set of five joint angles |
| Motion | An ordered set of keyframes |
| Keyframe | A target with travel time and arrival behavior |
| Anchor | The complete pose around which relative animation moves |
| Trajectory | Joint position, velocity and acceleration over time |
| Settling | Waiting for measured joints to remain near the final target |
| Scene | Motion, light and audio on one clock |

The [motion reference](motion-reference.md) defines fields, styles and compiler rules.

## From request to servo

```text
user, voice or Studio request
                ↓
character or scene ownership check
                ↓
named poses or anchor-relative offsets
                ↓
Rust trajectory compiler
                ↓
50 Hz runtime: read → sample → write
                ↓
STS3215 driver or MuJoCo driver
```

Ordinary movement starts from measured position and velocity. Speech continuation starts from the current commanded state to preserve continuity despite servo lag.

Absolute motions resolve named poses. Relative motions add offsets to the anchor. The compiler reduces relative amplitude uniformly when necessary. This preserves the gesture's joint proportions.

The compiler connects keyframes with quintic curves. A quintic curve is a fifth-degree polynomial with position, velocity and acceleration boundary conditions. Through arrivals preserve flow. Settle arrivals end with zero velocity and acceleration.

The compiler limits overshoot and adjusts timing for motor speed. The driver checks every goal against command limits. Runtime completion depends on measured settling after the timeline ends.

## Character mode

### Shared mind state

`oriond` owns energy, mood, activity and attention in `expression/mind.rs`. The agent and Studio read this state.
The state does not grant motion ownership, start detection or change wake confirmation.

Energy combines local time and conversation activity. Each confirmed turn increases activity. Activity falls with a thirty-minute half-life.
Mood is a value from minus one to one. Spoken reaction cues adjust it. Mood falls toward zero with a twenty-minute half-life.
The state label is `sleepy`, `cheerful`, `subdued`, `curious` or `content`.

Attention uses existing speaker, person, Hands and object targets. Its timestamp changes when the target changes.
A worker reads the local clock and saves state. The control loop uses arithmetic and a bounded message queue.
See [saved files](configuration.md#saved-files) for storage and reader rules.

### Startup and anchors

Character startup applies configuration, enables torque and moves to `home`. Successful measured arrival establishes the idle anchor. Failed or cancelled startup leaves character mode off.

Installed services start character mode. Manual V2 serving defaults to maintenance mode. See [startup configuration](configuration.md#runtime-startup).

```text
Off → Starting → HomeIdle / PoseIdle
                      ↓
             Listening / Thinking / Speaking
                      ↓
                   Settling
                      ↓
              HomeIdle / PoseIdle
```

A held pose becomes the anchor for idle and speech motion. Relative animation returns to that anchor. An explicit pose or completed foreground scene can replace it.

Character Stop returns home and leaves holding torque enabled. Mechanical rest uses a separate coordinator that releases torque after descent. See [rest and waking](system-architecture.md#automatic-rest-and-waking).

## Movement priority

Higher-priority ownership can cancel or suppress lower-priority work.

| Priority | Owner |
| --- | --- |
| Highest | Shutdown, cancellation and torque release |
| Foreground | Explicit scenes and motions |
| Speech | Reply playback and speech gestures |
| Attention | Voice attention, gaze, listening and thinking |
| Background | Idle motion and idle lighting |

Scene, speech and character coordinators plan their work. `RuntimeCore` executes all joint movement.

## Autonomous behaviors

Optional visual behaviors share one curiosity budget. The budget limits gesture frequency and repeated targets. It uses bounded memory without identifying people.

Privacy off, Tracking Off, Stop, rest, voice and foreground work close admission. Active light routines and alerts also close admission. The runtime has no separate clock-based quiet-hours schedule.

Object glances, mutual gaze, arrival notice and new-object peeks require their saved behavior switches. Failed execution does not refund a reserved gesture. Camera changes do not reset the budget.

### Presence defers automatic rest

The `presence_defers_rest` option can delay automatic descent while fresh face or hand observations continue. It keeps the original inactivity deadline. When presence ends, an expired deadline can start rest immediately.

Presence does not wake Orion, enable torque or reset voice confirmation. Explicit rest and sleep retain priority. Presence deferral consumes no movement budget.

### Idle tracking

Tracking Idle publishes observations without ambient face following. A confirmed conversation can follow its selected speaker. It does not replace a missing speaker with a bystander.

Tracking mode differs from Idle/Lamp routine mode. See [camera modes](../vision/README.md#tracking-modes).

### Hands following

Hands mode keeps the current pose as its held anchor. The head aims the camera and light toward a hand point on the desk. Hands and Faces share the gaze solver, angular deadband, head steps, speed limit and return settle. Hands keeps base yaw fixed.

Without a hand point, the head tilts down 30 degrees from the held pose. The shoulder, elbow and neck keep their held angles. The lamp holds this search direction while it waits for a hand.

The arm moves only when the head cannot reach the hand point. The planner searches bounded shoulder and elbow offsets for the smallest reachable lean. `--hands-max-arm-degrees` sets the maximum offset of each arm joint, from zero to five degrees. The default is five degrees. This option does not change calibration or joint limits.

The arm returns when the hand enters head-only reach or leaves. After ten seconds without a hand, the head returns to its held angle. A two-second camera timeout or a tracking mode change also requests this return. Idle motion waits while Hands owns tracking.

Hand presence cannot wake a resting lamp or clear Stop. Explicit movement releases the held Hands anchor. A movement failure keeps its diagnostics until a successful return or torque disable.

### Object aims

An explicit desk-object aim can establish a held work anchor. Face gaze leaves that anchor unchanged. Explicit movement, character restart or selecting Idle routine mode releases it.

## Failure behavior

Invalid targets fail before execution. Driver or movement failures cancel affected work. A failed rest or wake movement enters the rest fault state. Runtime status exposes the failure reason.

Hardware and simulation share compilation and lifecycle checks. V2 simulation follows kinematic targets; it does not model complete lamp dynamics.

## Change an animation

1. Select the relevant hardware version.
2. Edit a named pose or motion using the [schema](motion-reference.md#pose-schema).
3. Preview the action in [MuJoCo](../simulation/mujoco/README.md).
4. Inspect the compiled range and timing before hardware playback.

WARNING: Playback moves powered joints. Keep the movement area clear.

5. Run the action through the owning runtime.
