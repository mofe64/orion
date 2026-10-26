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

Ordinary movement starts from measured position and velocity. Speech continuation and gaze start from the current commanded state. Servo lag does not change their commanded starting position.

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

Energy adjusts both idle delays, head movement variation, tempo and background light. Low energy gives a warmer background light.
Hands and held aim anchors retain their existing rhythm and light behavior. Motion remains intermittent.

Listening can hold `listen_lean_in` around the current anchor. Processing releases the lean. Speech replaces its remaining path through commanded-state continuation.
After three seconds of thinking, small beats alternate at intervals of three to five seconds. Speech takes priority over these beats.

Posture drift selects `home_low`, `home_attentive`, `home` or `home_window` from the shared state. Drift selects `home_window` only when no person is in view.
It requires an awake idle character without conversation, foreground work, Hands, held aim or pending rest.
Each move takes at least three seconds and uses calibrated compilation and comfort checks. Successful measured arrival commits the new anchor.
Stop, explicit movement and Hands can cancel drift. The behavior journal records each started move and its outcome.

Automatic rest returns a drifted posture to home before descent. The return requires successful measured arrival.
`sleep_settle` then lowers the head with a dimming light before the existing rest descent. A failed act still permits that descent.
After confirmed wake reaches home, `wake_stretch` lifts and settles. A failed stretch still permits the existing wake sequence.
Only successful measured completion of the rest descent permits torque release.

Low energy can select `yawn` when automatic rest is less than two minutes away.
Wake, sleep and yawn markers can play short sounds. Routine idles, posture drift, listening and thinking remain silent.
Speech, active alerts, light routines and local quiet hours suppress these sounds. Quiet hours run from 22:00 to 07:00.
Sound failure does not control motion or rest. See [character switches](configuration.md#character-behaviors).

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

### Situation and initiative

`oriond` builds one situation for each camera update. The situation contains:

- The number of faces in view, and if a face looks at Orion.
- Focus: a person looks away from Orion for a long time.
- The visit: when it started, and since when nobody is in view.
- The local hour and day.
- Busy: a voice turn, a reply, music, an alert, a light routine or a scene is active.
- Orion's own light: on or off, brightness and mood.
- Room light in lux, when the camera gives it.
- Liveliness: Quiet, Lively or Demo, from `proactive_speech` and `demo_mode`.

A visit starts when a face comes into view. It ends when nobody is in view for 10 min, or 30 s in Demo. When `oriond` starts, it treats the current visit as already started.

All reactions use one initiative path. A reaction asks the curiosity budget for admission. If the budget admits it, the path stops idle motion, plays the reaction, records it in the behaviour log and offers its remark. Remarks read the same situation.

### Quirks

Each Orion has quirks in `personality.json`. A quirk changes what Orion does:

| Quirk | Value | Effect |
| --- | --- | --- |
| Curiosity | Shy | 2 gestures in 10 min, 2 min apart, 4 min before the same target again. One idle glance in four looks at an object. |
| Curiosity | Normal | 3 gestures in 10 min, 1 min apart, 2 min before the same target again. One idle glance in three looks at an object. |
| Curiosity | Nosy | 5 gestures in 10 min, 30 s apart, 1 min before the same target again. One idle glance in two looks at an object. |
| Chattiness | Reserved | Lively remarks at least 20 min apart, at most 2 in one hour. Very short remarks. |
| Chattiness | Normal | Lively remarks at least 10 min apart, at most 4 in one hour. |
| Chattiness | Chatty | Lively remarks at least 5 min apart, at most 6 in one hour. Remarks can add a playful thought. |

Each remark request contains the quirks, so the reply model speaks in the same style. Demo ignores quirk limits.

### Curiosity budget

Optional visual behaviors share one curiosity budget. The budget limits gesture frequency and repeated targets. It uses bounded memory without identifying people.

Privacy off, Tracking Off, Stop, rest, voice and foreground work close admission. Active light routines and alerts also close admission. The runtime has no separate clock-based quiet-hours schedule.

Object glances, mutual gaze, arrival notice and new-object peeks require their saved behavior switches. Failed execution does not refund a reserved gesture. Camera changes do not reset the budget.

### Spoken remarks

With `proactive_speech` or `demo_mode`, an arrival, a sustained look or a new-object peek can offer one spoken remark. The movement starts first. The voice listener then opens a session without a wake word. The agent writes the remark from the event and the recent remarks. It cannot use tools for a remark. An empty reply or `SILENT` gives no speech. After a remark, the user can reply without the wake word.

The runtime controls when a remark can start:

- Each event type has a cooldown. Arrival is 30 min, sustained look is 20 min and new object is 60 min.
- Remarks are at least 10 min apart, or the time that the chattiness quirk sets. Each ignored remark doubles this time, to a maximum of four times. A reply resets it.
- At most 4 remarks occur in one hour, or the number that the chattiness quirk sets.
- Orion speaks to the people in view one time each visit, with a welcome, a remark about a look or a conversation. See the visit in [Situation and initiative](#situation-and-initiative). People in view when `oriond` starts get no welcome.
- A remark waits 5 min after a conversation.
- Common desk objects get no remark. Each object category gets a maximum of one remark each day. A new object must stay in view for 12 s.
- No remark starts during a conversation, speech, music, scenes, focus, rest, Stop, or between 22:00 and 07:00.
- A request older than 8 s is discarded.
- The request "be quiet for a while" stops remarks for up to 8 hours.

A wake word during a remark stops the remark and starts a normal turn.

### Lively behaviors

In Lively, Orion also cares for the people at the desk. Each behavior acts only while a face is in view. Remark rules apply to the words.

| Behavior | When | What Orion does |
| --- | --- | --- |
| Wave back | A palm is raised near a face for 0.6 s | Plays `wave_back`: the head turns to the face, the arm lifts and sways about 14° to each side two times, and the head nods. The curiosity budget and the eye contact switch apply. Orion does not speak. |
| Light care | The room is below 15 lux for 60 s, and Orion's light is off or below 25% | Turns to the person and offers to turn the light on. The reply model knows which light people usually choose in the evening. |
| Break nudge | A person is at the desk for 90 min. Absences shorter than 5 min do not stop the count. | Plays `wake_stretch` and suggests a short break. |
| Goodnight | A farewell occurs between 21:00 and 05:00 | Says goodnight instead of goodbye. If nobody comes back in 120 s, automatic rest starts. Lamp mode keeps Orion awake. |

Light care and break nudge occur one time each visit. They do not use the curiosity budget, and focus does not stop them. A goodnight can occur at night and directly after a conversation. Nobody is expected to reply to a goodbye or a goodnight, so no reply does not increase the wait for the next remark.

A welcome can ask about something to come back to. See [home memory](configuration.md#home-memory). In Demo, the break nudge comes after 60 s at the desk and light care after 5 s of darkness. Wave back does not occur in Demo, because a raised palm starts hand play.

### Demo mode

Demo mode is for showing Orion to people. It changes these limits:

- The curiosity budget allows 20 gestures in 10 min, 12 s apart, without longer waits for repeated targets. Turning demo mode on clears the budget.
- Remarks can be 30 s apart, with a 45 s cooldown per event type and 20 s after a conversation. A new visit starts when nobody is in view for 30 s. The clock, focus and daily object limits do not apply.
- Arrival needs 30 s of absence instead of 10 min.
- A sustained look plays `mutual_gaze_notice_demo`, which has a nod and a larger neck turn.
- Each remark sends one camera picture with the request.

Two reactions occur only in demo mode:

- Farewell: a person who stayed for at least 20 s and is then gone for 5 s gets `farewell_watch` and a goodbye. The head turns to the last face bearing and gives a small sigh.
- Hand play: in Idle tracking, vision runs palm detection. A palm in view for 1 s sets tracking to Hands and offers a remark that invites play. When no palm is visible for 4 s, tracking returns to the earlier mode.

### Presence defers automatic rest

The `presence_defers_rest` option can delay automatic descent while fresh face or hand observations continue. It keeps the original inactivity deadline. When presence ends, an expired deadline can start rest immediately.

Presence does not wake Orion, enable torque or reset voice confirmation. Explicit rest and sleep retain priority. Presence deferral consumes no movement budget.

### Idle tracking

Tracking Idle publishes observations without ambient face following. A confirmed conversation can follow its selected speaker. It does not replace a missing speaker with a bystander.

Tracking mode differs from Idle/Lamp routine mode. See [camera modes](../vision/README.md#tracking-modes).

### Hands following

Hands and Faces use the same gaze planner. Vision sends a filtered image bearing for the selected target.

The neck leads the turn. The base follows to center the neck. The shoulder and elbow keep the held anchor.

A lost target follows the same settle and anchor return in both modes. A new target replans from the current commanded state.

Hand presence cannot wake a resting lamp or clear Stop. Servo and bus faults use the shared runtime handling.

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
