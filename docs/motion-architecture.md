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
4. Voice attention, bearing gaze, and listening or thinking reactions.
5. Autonomous idle.
6. Background idle lighting.

The scene coordinator owns scene tracks, the speech coordinator owns audio
playback, and the character coordinator owns the anchor, idle schedule and
generated speech motion. `RuntimeCore` is the only thing that executes
movement for all three.

### Autonomous vision restraint

`CuriosityBudget` supplies a shared admission policy for autonomous vision
gestures. It permits at most three in a rolling ten-minute window, separated
by at least sixty seconds. Repeating a target extends its cooldown from two
minutes to four, eight, sixteen and then thirty-two minutes. An hour without
a reaction forgets that target's history. Target keys describe local tracks
and objects; they are not biometric identities. Camera or tracking changes
must not reset the budget.

Two minutes of continuous, fresh observations of a person looking away
suppress gestures except glances. A gap over two seconds, a frontal face or
an unknown observation breaks that focus inference. It is a coarse restraint
signal, not proof of where someone is looking. Glances still spend the shared
budget. Camera privacy, Tracking Off, Stop, rest, voice, foreground work,
disabled settings and quiet hours close admission. A caller reserves budget
after checking its candidate and motion; failed execution does not refund it.

The policy has bounded in-memory state and no detector, motion or servo IO.
Object idle glances, mutual gaze, arrival notice and object peeks use this policy.
Hands mode also uses this policy. Presence deferral admits no gesture and does
not spend motion budget. The runtime closes admission during active lamp routines (including
wind-down and sunrise), alerts and rest; it has no independent clock-based
quiet-hours schedule.

### Presence defers automatic rest

With `presence_defers_rest` enabled, fresh face observations in Faces mode or
palm observations in Hands mode can keep an already powered, awake lamp from
starting automatic rest. Evidence expires after two seconds; camera privacy,
Tracking Off, an empty observation or disabling the preference removes it.
Object detections and pets do not count. Only the active detector's presence
field is considered, without identifying a person or storing an image.

The original inactivity deadline is retained. While it is expired and fresh
presence holds it, `rest.presence_deferred` is true and `remaining_seconds` is
zero. Once presence ends or becomes stale, automatic rest can begin immediately
if no foreground owner or conversation is still deferring it. Presence never
refreshes voice confirmation, queues a wake, changes routine mode or enables
torque. Explicit rest, sleep-after-reply and scheduled wind-down retain priority.
An already descending or resting lamp ignores presence as a wake signal.
This policy spends no curiosity budget because it initiates no movement; it
only inhibits an automatic transition. Lamp checks remain owed for sustained
work, departure, privacy, explicit rest and absence of surprise waking.

### Hands following

Hands is an explicit tracking mode, separate from behaviour switches and the
Idle/Lamp routine mode. One inference worker runs the palm detector at five
frames per second; YuNet and background YOLO are excluded. Explicit `/find`
retains its voice-safe ownership interval and pauses whichever detector was
selected. `/speaker` returns `503 tracking_off`, so V2 conversation stays at
the work anchor instead of turning or scanning for a face.

The first fresh Hands observation can enter `desk_work` through `hands_entry`.
The authored anticipation and arm lead are adapted to the measured start;
shoulder and elbow lead before the camera pitches down. Completion establishes
the desk pose as the character anchor. The `directional` idle profile avoids
outward yaw clips near range limits. Subsequent generated idle/speech drawings
are projected into the coupled comfort box, use synchronous settle curves and
are checked over their complete trajectory. This protection applies while a Hands or successfully aimed desk-light anchor
is held; it does not change ordinary home animation.

Measured starts may differ from the comfort hull by the runtime's 0.05-radian
completion tolerance. Planning projects that reference into the hull while
compiled output retains the actual measured start. Validation allows only that
initial error, decaying along the first quintic segment to zero at its first
settled keyframe. Later targets and curves must remain inside the original
hull; larger errors are refused. A generated clip rejected by validation is
logged and skipped, with `character.last_rejection` available in status, rather
than ending the daemon. Driver IO faults retain their existing handling.

Vision averages each frame's palm centres, projects that centroid using the
capture-time camera transform onto the desk plane, then averages these base-frame
points over two seconds. Transforming before averaging prevents lamp movement
from masquerading as work movement. Runtime requires an eight-centimetre shift
from the committed work point, stable within four centimetres for at least one
second. Gaps over two seconds reset this dwell. P8 IK with `lean=0.35` supplies a
slow `hands_follow` destination; no raw joint stream reaches the controller.
Only successful measured completion commits that work point and anchor.

Entry and each follow move spend the shared budget, including its sixty-second
spacing and three-per-ten-minute cap. Work-point keys use eight-centimetre cells
for habituation. Hands therefore follows deliberate changes sparingly; it is
not continuous hand chasing. Mode toggling cannot replenish the budget. Ten
seconds without hands stops scheduling ordinary idles while holding the work
anchor; an already running idle finishes its anchor return. Speech retains priority. Hands alone does not defer automatic rest.

Leaving Hands or disabling capture requests `hands_return` to the prior anchor.
The private `gaze hands off` heartbeat retries a deferred return after voice or
another temporary owner releases priority, even with no camera frames. Stale
inference also requests a bounded return. The runtime tick only checks lifecycle
and may compile a recovery; detector work, centroid projection and lean IK stay
outside it. A replacement preserves commanded position, velocity and acceleration;
failed preflight leaves the installed trajectory intact for a later retry.
Stop blocks following and recovery until an authorized resume. Explicit scenes,
pose commands and object aiming replace Hands ownership and require leaving and
reselecting Hands to resume following; old observations cannot undo those controls.
Rest clears ownership and never wakes from presence. All Hands behavior is
lamp-unverified, including own-light exposure, apparent work stability,
interruption, contact, shoulder load and CPU/ASR impact.

### Object peeks and pet notice

With `new_object_peek` enabled, background YOLO uses the same once-per-minute
pass as object glances. The first successful pass seeds a baseline, including
an empty view; enabling the preference does not make existing objects novel.
A later unmapped object with confidence at least 0.6 becomes one candidate.
Forward/backward optical flow follows at most 24 image features on the single
inference worker for up to twelve seconds, without another detector pass.
Three seconds of continuous observations are required. Feature loss, a gap over
one second, face overlap, voice or camera/mode changes discard the evidence.
All pixels remain in memory. Textureless objects can be missed deliberately.

An ordinary desk object supplies a capture-time table-plane point to lean IK
with `lean=0.8`. The compiled `peek_new_object` variant pulls back slightly,
starts the shoulder and elbow, follows with the head, adds a small curious
neck swivel and returns to the unchanged character anchor. Every drawing and
the entire curve must fit calibration and the coupled comfort box. Unsafe or
unreachable candidates are consumed without movement. Geometry and compilation
run when metadata arrives, outside the control tick.

Cats and dogs instead use the `delight_lift` target in a bounded `pet_notice`
variant. After compression and lift, fresh bearings can replace its owned
trajectory for three seconds, with a four-degree deadband and no more than two
updates per second. The neck leads and the base catches up. Each replacement
keeps the original anchor tail; the whole episode spends one budget admission.
Foreground work, speech, privacy, Stop, rest and stale observations retain
priority. Losing the tracked candidate requests the shared continuous return.
The complete episode has a twelve-second deadline plus any validated return.

Peeks and object glances share a class-and-quantized-base-bearing interest key;
local object IDs still distinguish consecutive targets. This is approximate
object association, not recognition of a particular physical item. Clearing
the map does not reset habituation. Blocked events are not replayed later.
The feature tracker and committed reaches are lamp-unverified; check shoulder
load, contact, return continuity and CPU/ASR performance before leaving them on.

### Arrival notice

With `arrival_notice` enabled, a face appearing after ten minutes of continuous
empty observations can receive a silent attentive entry and a warm light lift.
A camera gap over two seconds, privacy change or tracking change resets the
empty-room evidence. Startup and re-enabling a preference never imply an empty
room. The arrival is consumed even when higher-priority activity prevents the
response; it is not replayed later.

The head turns first using the observed base-frame bearing, then the arm follows
into the authored attentive posture. The planner projects its drawings into the
comfort box and validates the complete calibrated trajectory, including the
return to the unchanged character anchor. The shared budget and the same
`face:TRACK_ID` habituation key used by mutual gaze govern admission. A brief
three-second intensity lift accompanies the existing warm background effect.
There is no audio. Manual light, scenes, voice, Stop and rest retain priority.

Arrival notice never enables torque or wakes a resting lamp. An automatic-rest
timeout of ten minutes or less can put the lamp to rest before an arrival becomes
eligible, suppressing the welcome. The default timeout is thirty minutes; Lamp
mode disables that timeout. Privacy and
mode changes use the same owned return as mutual gaze. This behaviour is
lamp-unverified.

### Mutual gaze

With `mutual_gaze` enabled, 1.5 seconds of continuous frontal-face observations
can admit one silent response. YuNet landmark symmetry is a coarse cue; it is
not an eye-gaze measurement. The response keeps one track for an eight-second
observation window. It makes a small head-pitch adjustment, sometimes adds a
curious neck turn, and can add a tiny acknowledgement after another two seconds
of looking. If that person turns away or disappears during the window, a small
look-away becomes eligible after 0.6 seconds, once the owned motion can transition.

The complete bounded episode spends one shared curiosity reservation. Repeated
looks use the same face-track habituation key; toggling settings cannot refill
it. New faces do not replace the chosen person mid-episode. Motion centres on
the held gaze posture, while its final keyframe returns to the unchanged
character anchor. Detector metadata prepares and compiles each continuation at
command time; no detector or IK runs in the servo control tick. Calibration and
the coupled comfort box validate the whole curve, including position, velocity
and acceleration inherited from the preceding drawing.

Privacy, stale observations, Tracking Off and disabling the setting request an
owned anchor return. If shortening the proven tail would overshoot the comfort
boundary while braking, its existing validated anchor return finishes instead.
No further acknowledgement is appended. Wake, speech, rest, Stop and foreground
commands retain priority and never reset the budget. This behaviour is
lamp-unverified.

### Object-directed idle glances

With `object_glances` enabled, vision may run one YOLO pass per minute while
runtime is idle, awake and available, with no voice session. YuNet and YOLO run
sequentially on the same worker; the camera publisher and watchdog continue.
The capture-time camera transform is read before YOLO. People and objects
whose boxes overlap a face in that frame are excluded. At most 32 class-labelled
base-frame bearings remain in memory, expiring ten minutes after last sighting.
No image or object map is persisted.

`gaze surroundings` carries a bounded metadata snapshot to runtime. Runtime
validates its clock and prepares an object-directed variant of `idle_micro_glance`
through calibrated camera FK and the ordinary compiler at command time. It changes
only the existing clip's yaw/neck direction within its authored envelope. An
already-due idle may use that variant on every third eligible glance, provided
the shared budget permits it. It never selects the preceding object again.
Idle deadlines, clip selection, tempo, amplitude variation and final anchor
remain unchanged; unsuitable or absent objects leave ordinary random idles.
Directional pose profiles retain their existing yaw-safe clips.

Camera off, Tracking Off, a setting change or stale metadata invalidates the
prepared variant. An active object glance takes a continuous compiled return
to its anchor. Foreground movement, wake, speech, Stop and rest retain their
existing ownership and interruption paths. No autonomous code clears the Stop
latch. This integration is lamp-unverified.

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

V2 servers enable bearing gaze by default; `--gaze off` disables it. V1 and
non-serve operations leave gaze off. Camera geometry loads once at startup;
a load failure leaves the daemon serving with gaze unavailable and its error
visible in status. The vision service sends a latest
base-frame target, calculated with the measured pose at exposure time.
`GazeCoordinator` solves neck swivel and wrist pitch through URDF FK, then
generates anchor-relative keyframes. The neck leads; the base follows after
200 ms and the neck counter-rotates through the same geometric solve.

At most ten times per second, gaze extends the existing character spline from
its commanded position, velocity and acceleration. The final keyframe always
has zero offsets and settles at the anchor. A lost target stops extensions
after one second; the last plan holds briefly and returns. Calibration and
settle checks remain in the trajectory compiler and runtime.

Foreground work, speech, rest and Stop preempt gaze. Stop and disable latch
gaze off with `state:"stopped"` and the originating `stop_command`. A successful
`character start` clears either latch. Current-session wake confirmation clears
only a Stop latch on an already powered, enabled character with startup complete.
`gaze resume` requires existing torque; a disable latch needs explicit recovery.
These events cannot override configured-off or unavailable gaze. Targets, clears,
idle time, returning vision and stale wake sessions cannot undo Stop.
Confirmed-wake recovery rearms attention before bearing delivery without startup
or torque commands. Explicit character start after disable follows the measured
home lifecycle. Powered foreground work retains ownership. Gaze can replace lower
priority idle or thinking motion and suppresses idle while its run settles.
Streamed idle gaze never wakes a resting lamp, enables torque or moves the anchor.
Confirmed conversation tracking may update the temporary speaker-facing anchor
between replies; speech freezes it until the reply ends. The
[runtime command contract](../runtime/README.md#bearing-gaze-v2) and
[tuning values](motion-reference.md#bearing-gaze-tuning) define the interface.
Continuous conversation retargeting remains lamp-unverified; native MuJoCo
validates software tracking and calibrated samples.

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
