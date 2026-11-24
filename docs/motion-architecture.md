# How Orion moves

Orion turns named poses, motions and spoken interactions into smooth joint commands at 50 Hz. The Rust runtime (`oriond`) owns all motor execution for both physical hardware and the MuJoCo simulator, while voice, vision and Studio request movement through checked interfaces.

## Key terms

| Term | Meaning |
| --- | --- |
| Pose | A named set of all five joint angles |
| Motion | An ordered sequence of keyframes |
| Keyframe | A target pose or offset with travel duration and arrival style |
| Anchor | The reference pose around which relative gestures move |
| Trajectory | Planned joint positions, velocities and accelerations over time |
| Settling | Waiting for measured joints to stay near the final target |
| Scene | Motion, light and audio coordinated on one clock |

For exact file fields, styles and compiler rules, see the [motion reference](motion-reference.md).

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

Ordinary movements start from the lamp's measured joint positions and velocities. Streaming speech replacements and gaze updates start from the current commanded state instead, so normal servo lag does not pull the planned path backward mid-gesture.

Absolute motions move directly to named poses. Relative motions add offsets to the current anchor; if an offset would push any joint past its limit, the compiler scales the entire gesture down uniformly so the movement keeps its shape.

Keyframes are joined with quintic curves—fifth-degree polynomials matched in position, velocity and acceleration at both ends. A `through` arrival flows smoothly past a waypoint without stopping, while a `settle` arrival brings velocity and acceleration cleanly to zero. The compiler also limits unintended overshoot, stretches segments that exceed the STS3215 speed ceiling, checks every 50 Hz sample against joint limits, and waits for measured settling before marking a run complete.

## Character mode

### Shared mind state

`oriond` tracks Orion's internal state (`energy`, `mood`, `activity` and `attention`) in `expression/mind.rs`, where the agent and Studio can read it. This state lightly shapes tone and idle pacing, but never grants movement ownership, triggers detection or changes wake confirmation.

- **Energy** blends the time of day with recent conversation activity. Each confirmed turn boosts `activity`, which fades with a 30-minute half-life.
- **Mood** runs from `-1` to `1`. Spoken reaction cues nudge it up or down, and it drifts back toward zero with a 20-minute half-life.
- **State label** summarizes the state as `sleepy`, `cheerful`, `subdued`, `curious` or `content`.
- **Attention** tracks the active speaker, person, hand or object target and timestamps each target change.

A background worker reads the local clock and saves state to disk while the 50 Hz control loop relies only on fast arithmetic and a bounded message queue. See [saved files](configuration.md#saved-files) for storage and reader rules.

Energy scales idle intervals, head variation, tempo and background light warmth (low energy produces a warmer ambient light). Hands mode and held desk-aim anchors keep their own rhythm and lighting while remaining intermittent.

During conversations, Orion leans in slightly while listening (`listen_lean_in`) around its current anchor, then releases the lean once processing begins so speech gestures can take over smoothly. If a reply takes more than 3 seconds, alternating thinking beats play every **2.6 to 3.6 seconds** until speech starts.

During quiet awake idle periods (no conversation, foreground scene, Hands mode, held work anchor or pending rest), posture drift can occasionally move between `home_low`, `home_attentive`, `home` and `home_window` (which is chosen only when no person is in view). Drift moves take at least 3 seconds, pass calibrated comfort checks, commit the new anchor on measured arrival, and are recorded in the behaviour journal. Stop, explicit movement or Hands mode cancels drift immediately.

Automatic rest returns any drifted posture to `home` before descending. When enabled, `sleep_settle` lowers the head and dims the light before the rest descent, and `wake_stretch` stretches up after a confirmed wake reaches `home`. If low energy meets an inactivity deadline within two minutes, Orion may play `yawn` beforehand. Even if any of these decorative acts fail, the safe rest and wake sequences still run, and torque is released only after measured arrival at `rest`.

When a voice turn ends without an answer, Orion shows it in its body: `speak_react_unsure` plays if nothing was heard, and `speak_react_sorry` plays if the request failed. Each clip waits until the body holds still and expires if it cannot start within 2.5 seconds.

Short, voice-like sound cues can play at the markers of wake, sleep, yawn, thinking, arrival, peek and wave clips, while routine idles, posture drift and listening stay silent. Speech, active alerts, light routines and local quiet hours (22:00 to 07:00) suppress these sounds, and a sound playback failure never blocks motion or rest. See [character behaviors](configuration.md#character-behaviors) for the switches.

### Startup and anchors

Character startup applies saved settings, enables servo torque and moves to `home`. Once measured arrival succeeds, `home` becomes the idle anchor; if startup fails or is cancelled, character mode stays off. Installed Pi services start character mode automatically, while manual V2 serving defaults to maintenance mode (`--character-on-start off`; see [startup configuration](configuration.md#runtime-startup)).

```text
Off → Starting → HomeIdle / PoseIdle
                      ↓
             Listening / Thinking / Speaking
                      ↓
                   Settling
                      ↓
              HomeIdle / PoseIdle
```

Any held pose becomes the new anchor for idle and speech motion until replaced by another explicit pose or completed foreground scene. When Orion is holding a desk-light work anchor, routine idle motions pause so the light stays steady on your work.

Character Stop returns the lamp to `home` and keeps holding torque on. Mechanical rest uses a separate coordinator that releases torque only after reaching `rest`; see [rest and waking](system-architecture.md#automatic-rest-and-waking).

## Movement priority

Higher-priority owners take control immediately and cancel or suppress lower-priority work, while lower-priority requests are turned away until the lamp is free.

| Priority | Owner |
| --- | --- |
| Highest | Shutdown, cancellation and torque release |
| Foreground | Explicit scenes and motions |
| Speech | Reply playback and speech gestures |
| Attention | Voice attention, gaze, listening and thinking |
| Background | Idle motion and idle lighting |

Scene, speech and character coordinators plan gestures in their own layers, while `RuntimeCore` arbitrates ownership and executes every joint command.

## Autonomous behaviors

### Situation and initiative

On every camera update, `oriond` builds one shared `Situation` snapshot so every autonomous behavior reads the same picture of the moment:

- Faces in view and whether anyone is looking at Orion
- Focus (whether a person has looked away for an extended stretch)
- Visit start time and how long nobody has been seen
- Local hour and calendar day
- Whether Orion is busy (active voice turn, reply, music, alert, light routine or scene)
- Orion's own light state (on/off, brightness and mood) and room brightness in lux
- Liveliness setting (`Quiet`, `Lively` or `Demo`, set by `proactive_speech` and `demo_mode`)

A visit starts when a face enters view and ends after nobody has been seen for 10 minutes (30 seconds in Demo). On startup, `oriond` treats an ongoing visit as already active so it does not greet people who were already sitting at the desk.

Every social reaction goes through one shared initiative path: ask the curiosity budget for permission; if admitted, stop idle motion, play the gesture, log the decision and optionally offer a spoken remark.

### Quirks

Each lamp stores two quirks in `personality.json` that give it distinct pacing and speech habits:

| Quirk | Value | Effect |
| --- | --- | --- |
| Curiosity | `shy` | 2 gestures in 10 min, 2 min apart, 4 min before repeating a target; 1 in 4 idle glances looks at an object |
| Curiosity | `normal` | 3 gestures in 10 min, 1 min apart, 2 min before repeating a target; 1 in 3 idle glances looks at an object |
| Curiosity | `nosy` | 5 gestures in 10 min, 30 s apart, 1 min before repeating a target; 1 in 2 idle glances looks at an object |
| Chattiness | `reserved` | Lively remarks at least 20 min apart, at most 2 per hour; very short remarks |
| Chattiness | `normal` | Lively remarks at least 10 min apart, at most 4 per hour |
| Chattiness | `chatty` | Lively remarks at least 5 min apart, at most 6 per hour; may add a playful thought |

Remark prompts include Orion's quirks so the language model matches the body's temperament. Demo mode overrides quirk limits.

### Curiosity budget

Optional visual behaviors share one curiosity budget so Orion does not look twitchy when things move around the desk. Privacy off, Tracking Off, Stop, rest, voice activity, foreground scenes, active alerts and light routines all close admission, while clock-based quiet hours do not block quiet movement on their own.

Object glances, mutual gaze, arrival notices and new-object peeks each require their individual behavior switches. Failed moves do not refund the budget, and toggling the camera does not reset it.

### Spoken remarks

When `proactive_speech` (Lively) or `demo_mode` is enabled, an arrival, sustained look or new-object peek can trigger a short spoken remark after its movement starts. Orion opens a voice session without a wake word; the agent generates a remark (with no tools allowed, and may return `SILENT` to stay quiet) and then listens briefly so you can reply naturally.

In Lively mode, remarks obey these safeguards in order:

- **Per-event cooldowns:** 30 minutes for arrivals, 20 minutes for sustained looks, and 60 minutes for new objects.
- **Spacing and backoff:** At least 10 minutes between remarks (or the chattiness quirk's interval). Each ignored remark doubles that wait up to four times; replying resets it.
- **Hourly cap:** At most 4 remarks per hour (or the chattiness quirk's limit).
- **Once per visit:** Orion initiates conversation with someone at most once per visit (arrival welcome, look remark or conversation), and skips welcomes for people already in view at startup.
- **After conversations:** Waits at least 5 minutes after a normal conversation ends.
- **New objects:** Ignores everyday desk items (such as laptops, cups or keyboards), requires an object to stay in view for 12 seconds, and mentions each object category at most once per day.
- **Busy, focus and quiet hours:** Never speaks during conversations, speech, music, scenes, deep concentration, rest, Stop or between 22:00 and 07:00.
- **Freshness and mute:** Drops queued remarks older than 8 seconds, and saying `"be quiet for a while"` silences remarks for up to 8 hours.

Saying `"Hey Orion"` during a remark interrupts it immediately and starts a normal voice turn.

### Lively behaviors

In Lively mode, Orion also looks out for people at the desk (only while someone is in view, using the same remark rules for speech):

| Behavior | Trigger | What Orion does |
| --- | --- | --- |
| Wave back | A palm stays raised near a face for 0.6 s | Plays `wave_back` (turns to the face, lifts and sways the arm ~14° twice, nods) without speaking; respects the curiosity budget and eye-contact switch |
| Light care | Room is below 15 lux for 60 s and Orion's light is off or under 25% | Turns to the person and offers to turn its light on, using their usual evening light preference |
| Break nudge | Someone stays at the desk for 90 min (absences under 5 min do not reset the timer) | Plays `wake_stretch` and suggests taking a short break |
| Goodnight | A farewell happens between 21:00 and 05:00 | Says goodnight instead of goodbye and enters automatic rest if nobody returns within 120 s (unless Lamp mode is active) |

Light care and break nudges happen at most once per visit, skip the curiosity budget, and ignore focus detection. Because nobody is expected to answer a goodbye or goodnight, an unanswered parting does not trigger remark backoff. Welcomes can also mention upcoming events saved in [home memory](configuration.md#home-memory).

### Demo mode

Demo mode speeds everything up for showing Orion to a group:

- Curiosity allows up to 20 gestures in 10 minutes, 12 seconds apart, with no per-target backoff (toggling Demo on clears the budget).
- Remarks can run 30 seconds apart with 45-second event cooldowns and 20-second post-conversation pauses; visits reset after 30 seconds of absence, and clock, focus and daily-object limits are lifted.
- Sustained looks play the larger `mutual_gaze_notice_demo` clip, and every remark includes one camera snapshot so Orion can comment on what it sees.
- Two Demo-only behaviors activate: **Farewell** (plays `farewell_watch` and says goodbye when someone who stayed at least 20 seconds leaves for 5 seconds) and **Hand play** (in Idle tracking, holding a palm in view for 1 second switches tracking to Hands and invites play, returning after 4 seconds with no palm).

In Demo mode, break nudges trigger after 60 seconds, light care triggers after 5 seconds of darkness, and wave-back is disabled because a raised palm starts hand play instead.

### Presence defers automatic rest

When `presence_defers_rest` is enabled, fresh face or hand sightings postpone automatic descent without resetting the inactivity deadline—as soon as presence ends, an expired deadline starts rest immediately. Presence never wakes a resting lamp, enables torque or counts against the curiosity budget.

### Idle tracking

In `Idle` tracking mode, vision publishes observations without following bystanders. During an active conversation, Orion follows the confirmed speaker and never substitutes a bystander if they leave.

If the listener moves more than 10° away from where Orion is pointing mid-reply, Orion waits for the current gesture to finish (or up to 1.5 seconds), turns its neck and then its base toward them, and resumes speaking gestures in the new direction. It skips the turn if a reaction is due or the reply is nearly done. See [tracking modes](../vision/README.md#tracking-modes).

### Hands following

`Hands` and `Faces` share the same gaze planner: vision streams filtered bearings for the selected target, the neck leads the turn, the base catches up to re-center the neck, and the shoulder and elbow hold their anchor. Losing the target holds briefly and then settles back to the anchor; a new target replans smoothly from the current commanded state. Hand presence never wakes a resting lamp or clears Stop.

### Object aims

An explicit desk-object aim (`aim_light`) establishes a held work anchor. Ambient face gaze leaves that anchor undisturbed until an explicit move, character restart or routine mode change releases it.

## Failure behavior

Invalid targets fail before execution, and driver or servo errors cancel affected movement while surfacing the failure reason in runtime status. A failed rest or wake move enters the `fault` state and keeps Orion's light on. Hardware and MuJoCo share the same compiler and lifecycle checks, though V2 simulation tracks kinematic targets rather than modeling full lamp dynamics.

## Change an animation

1. Select the target hardware version (`v1` or `v2`).
2. Edit a named pose or motion following the [pose and motion schemas](motion-reference.md#pose-schema).
3. Preview the compiled result in [MuJoCo](../simulation/mujoco/README.md).
4. Inspect the joint range and timing before running on hardware.

WARNING: Hardware playback moves powered joints. Keep the movement area clear before playing a motion.

5. Run the pose or motion through `oriond`.
