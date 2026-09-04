# Character animation design

Orion should feel calm, curious, warm, and attentive: alive enough that a person can read intention,
restrained enough that movement never competes with the lamp's task or the conversation.

The governing rule is **one primary idea at a time**. A turn, nod, thought,
glance, or spoken phrase should have one readable lead action. Other joints,
light, and sound support that action with hierarchy and overlap.

## From animation principle to robot behavior

The traditional 12 principles describe screen animation. Orion translates
them into joint-space authoring and runtime rules.


| Principle                             | Orion interpretation                                                      | Implementation                                                                                                               |
| ------------------------------------- | ------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| Squash and stretch                    | Coordinated compression and extension;                                    | Shoulder and elbow close the silhouette before an opening or lift, and preserve safe joint geometry                          |
| Anticipation                          | A small, usually opposing preparation announces a larger action           | Expressive turns begin with an opposite yaw; lifts may begin with compact shoulder/elbow compression                         |
| Staging                               | One joint group and silhouette communicates the idea first                | A head-led speaking phrase keeps shoulder/elbow action secondary; scene light and sound land on the same dominant beat       |
| Straight ahead and pose-to-pose       | Authored semantic drawings are combined with procedural sequencing        | Expressive actions use named poses; idle selection and speech composition vary those approved motion shapes                  |
| Follow-through and overlapping action | Supporting parts continue or arrive after the lead                        | Head, base, shoulder, and elbow use different path character; speech explicitly delays the body after the head lead          |
| Slow in and slow out                  | Velocity and acceleration evolve continuously around intentional arrivals | The whole-motion quintic compiler shares derivatives at `through` drawings and reaches zero only at `settle`                 |
| Arcs                                  | Coordinated rotations produce curved lamp-head and body paths             | Base, shoulder, elbow, roll, and pitch are reviewed together in MuJoCo and on hardware, not one servo at a time              |
| Secondary action                      | Small detail reinforces the main idea                                     | A restrained elbow follow, counter-tilt, or warm light effect supports rather than becomes a second gesture                  |
| Timing                                | Pace communicates weight, attention, and energy                           | Named styles change tempo, tangent energy, lag character, amplitude, and settle weight                                       |
| Exaggeration                          | Important intent receives controlled contrast                             | Authored overshoot, phrase nods, and sparse explanatory body beats are stronger than ordinary movement but remain calibrated |
| Solid drawing                         | Every held pose has a balanced, readable silhouette                       | Complete five-joint poses are inspected from useful viewpoints and under gravity in MuJoCo and on the physical robot         |
| Appeal                                | Motion consistently expresses Orion's temperament                         | Asymmetry, warm multimodal cues, forward eyeline, and purposeful stillness replace generic robotic oscillation               |




## The character state machine

`oriond` starts character mode by default after daemon initialization. Studio
can stop it for the current daemon session; restarting the daemon enables it
again. `--character-on-start off` is the maintenance override. Startup uses the
ordinary configure, torque-enable and home movement sequence. Only successful
measured home completion enters idle; cancellation or timeout leaves character
mode off and retains the failed movement status for diagnosis.

```text
Off
  │ character start
  ▼
Starting ── home completed ──▶ HomeIdle
                                  │
                   held elsewhere ▼
                              PoseIdle
                 ┌───────────────┼───────────────┐
                 ▼               ▼               ▼
             Listening        Thinking        Speaking
                 └───────────────┴──────┬────────┘
                                        ▼
                                ForegroundScene
                                        │
                                        ▼
                                    Settling
                                        │
                                        ▼
                              HomeIdle / PoseIdle

Any enabled state ── character stop ──▶ ShuttingDown ──▶ Off
```

Starting character mode applies the servo profile if necessary, enables holding torque, moves to `home` pose, and captures a measured anchor. Stopping character mode cancels owned foreground and speech work, returns to `home`,
clears character lighting, and leaves powered holding torque on. Moving to
mechanical `rest` and releasing torque are separate, explicit operations.

The priority order prevents competing performances:

1. shutdown, cancellation, and release;
2. explicit foreground scene or motion;
3. speech;
4. listening or thinking reaction;
5. autonomous idle;
6. background lighting.



## Immutable anchors

An anchor is a complete measured joint pose captured when Orion enters an idle
context. Relative animation is always calculated from that immutable map.

Suppose an idle offsets the shoulder by `+0.05 rad`. The next idle does not add
its offset to the first idle's final measured value. Both are independently
resolved as:

```text
target[joint] = anchor[joint] + style_amplitude × uniform_scale × offset[joint]
```

Every idle and generated speaking performance ends with a zero-offset
`settle`, so its final target is exactly the anchor. This design prevents a
random walk and makes interruption recovery deterministic.

A successfully completed foreground scene may intentionally replace the
anchor with its final measured position. A direct foreground motion captures
the held measured position after its run ends. Speech, reaction state, and idle
never replace the anchor; failed or cancelled scenes do not either.

## Autonomous idle animation



### Scheduling

The coordinator owns two independent monotonic deadlines:

- a micro-idle after a seeded random delay from 8 to 20 seconds;
- a larger idle after a seeded random delay from 35 to 75 seconds.

Whichever deadline is earlier is the next category. Completing an idle
reschedules only that category, preserving the other deadline. Speech, a
foreground action, or a reaction-state change resets the schedule so Orion
does not immediately add ambient movement after a user-facing action.

The scheduler seeds its pseudorandom generator. Physical runs still vary,
while tests and previews can reproduce the exact selection sequence. The
scheduler removes the immediately preceding clip from the candidate set.

### Profile-aware selection

The nearest pose's `idle_profile` adapts ambient movement to the held
silhouette:

- ordinary powered anchors use breathing, head curiosity, micro glance,
shoulder adjustment, weight shift, and soft head shake;
- attentive anchors may add `idle_attentive_hold`;
- directional anchors use `idle_directional_hold` and avoid yaw clips that
would collapse against the left or right calibration boundary.

The two categories create contrast. Micro-idles are short details; larger
idles redistribute more of the body and happen less often.

### Safe amplitude and no drift

Before compiling a relative clip, the runtime computes the largest single
scale in `[0, 1]` that keeps every styled offset inside the live calibrated
range around the anchor. The runtime applies one scale to the whole clip. This
retains the authored relationship among joints instead of flattening whichever
joint reaches its limit first.

The motion starts from measured position and velocity but resolves every
target from the immutable anchor. If foreground work arrives, `stop` cancels
the idle lifecycle and the foreground trajectory blends from the measured
interruption state. There is no forced trip back to the anchor before the
action.

### Idle light and sound

The lowest-priority background light comes from the nearest anchor pose's
`default_lighting`; `warm_idle_breathe` is the fallback. Listening, thinking,
starting, and settling states select their corresponding restrained effects.

Routine idles do not play sound. Repetitive ambient audio makes autonomous
behavior feel like notification noise, while motion and low-intensity light
are sufficient to communicate life.

## Speech-driven animation

Speech is an utterance-length performance generated before its movement
begins. Orion does not play a short gesture, stop, choose another, and restart.
The trajectory compiler combines the complete sequence of phrase drawings into
one motion spline with one final settle.

### Audio ownership and analysis

Studio synthesizes a whole response and uploads a validated RIFF/WAV file to
the Pi. The gateway requires mono, 24 kHz, signed 16-bit pulse-code modulation
(PCM16). It applies size and duration limits, writes an atomic random spool
item, and asks the speech coordinator to play that identifier. The coordinator
never accepts an arbitrary path.

The runtime divides PCM into 20 ms frames, matching its 50 Hz loop. For each
frame it calculates root-mean-square (RMS) energy and applies exponential
smoothing:

```text
smoothed[n] = 0.65 × smoothed[n-1] + 0.35 × rms[n]
```

It derives:

- quiet regions below `max(12% of maximum energy, 0.004)`; the analyzer
discards internal runs shorter than three frames but retains a trailing quiet
run; and
- phrase peaks above `1.35 × mean energy`, locally maximal, and separated by
at least ten frames (200 ms).

The same analysis drives movement planning and the `speaking_energy` light.
The light has a faster attack than release, is capped below full brightness,
and remains secondary to physical acting.

### Planning the utterance

The character coordinator allocates the waveform duration between active
phrase motion and one final settle. It then plans phrase drawings with seeded
variation:

- ordinary phrases prefer `speak_calm_sway` and `speak_reflective_tilt`, with
occasional explanatory shapes;
- detected peaks prefer `speak_emphasis_nod` and explanatory shapes;
- immediate clip repetition is excluded;
- duration varies around the phrase category's nominal timing;
- head roll alternates direction and varies in magnitude;
- head pitch supplies nods, lifts, and counter-shapes;
- small base-yaw turns are chosen without repeating a direction and are
constrained away from a directional anchor's nearby limit.

The authored clips contribute approved character shapes. The generated
performance varies and layers those shapes rather than inventing unconstrained
joint targets.

### Head-led staging

Every planned phrase is divided into two `through` drawings:

1. **Head lead:** roll, pitch, and optional yaw establish the phrase direction
  while the body retains the preceding secondary shape.
2. **Body follow:** shoulder and elbow arrive later while the head blends
  toward the next phrase's arc.

The head lead receives roughly two-thirds of the phrase duration. During the
body follow, the head target includes an 18% look-ahead toward the following
phrase's head target. That staging creates anticipation and overlap inside the
same continuous spline.

The planner scales ordinary shoulder and elbow offsets to remain visibly
subordinate to the head. It allows a larger explanatory body beat only when all
of these conditions hold:

- the drawing is associated with a detected phrase peak;
- its peak energy is at least 72% of the utterance maximum;
- its drawing index is at least three greater than the preceding body beat
(at least two drawings intervene); and
- it would not immediately repeat an explanatory lean.

This is the movement hierarchy:

```text
head direction and eyeline          primary on every phrase
shoulder/elbow follow-through       visible secondary action
full explanatory body beat          sparse emphasis only
speaking-energy light               supporting state cue
audio                               timing source and semantic content
```

The performance ends with one zero-offset `settle` around the pre-speech
anchor. All internal drawings are `through`. There is no independent periodic
elbow oscillator and no scheduler gap between gestures.

### Implemented performance policy

These values are character policy, not hardware limits:


| Policy                                      | Implemented value                                     |
| ------------------------------------------- | ----------------------------------------------------- |
| Motion end lead before audio duration       | `0.12 s`                                              |
| Nominal final settle budget                 | `0.55 s`, bounded for short utterances                |
| Phrase-duration scale                       | `1.35`                                                |
| Ordinary phrase base duration               | `1.05 s` before scale, randomization, and style tempo |
| Emphasis phrase base duration               | `0.72 s` before scale, randomization, and style tempo |
| Duration randomization                      | `0.90–1.10`                                           |
| Ordinary head amplitude multiplier          | `0.88–1.10`                                           |
| Emphasis head amplitude multiplier          | `1.05–1.24`                                           |
| Ordinary yaw turn                           | `0.045–0.085 rad`                                     |
| Emphasis yaw turn                           | `0.070–0.110 rad`                                     |
| Ordinary body multiplier on source drawing  | `0.32–0.48`                                           |
| Full body-beat multiplier on source drawing | `0.78–0.96`                                           |
| Head-lead share of each phrase              | `0.64–0.74`                                           |
| Next-head look-ahead during body follow     | `0.18`                                                |
| Full body-beat energy gate                  | At least `0.72` of utterance maximum                  |
| Full body-beat spacing                      | Drawing-index difference of at least `3`              |


Seeded variation selects the value inside each range. The final compiled
motion may be uniformly reduced near calibration boundaries and may be retimed
to respect the motor-speed ceiling.

### Speech interruption and failure

Speech motion is best-effort: a movement-planning failure must not silence a
valid response. If movement cannot start, audio continues.

When playback ends before the generated movement, the runtime cancels the
performance and compiles a short anchor-relative settle from measured state.
Cancellation does the same. If audio upload, validation, synthesis, or playback
fails, the speech run becomes `failed`, the coordinator removes its temporary
file, and character motion returns to the anchor. Idle resumes only after the
scheduler chooses another randomized delay.

Studio's playback-complete acknowledgement may arrive while the physical
settle is still running. Listening, thinking, and neutral reactions preserve
the speaking or settling state until that movement is cleaned up. The next
speech waits only for a matching active movement; a run that has left the
runtime's bounded movement history cannot block a later performance.

## Designing animation

Before adding an asset, answer these questions:

1. What is the single primary idea and which joint group leads it?
2. What silhouette must be readable at the dominant drawing?
3. What anticipation makes the action understandable before it arrives?
4. Which joints follow through, and how are they kept secondary?
5. Which drawings flow through and where is a real settle justified?
6. Does the action trace a coordinated arc rather than isolated servo changes?
7. How does timing express weight and temperament?
8. What calibrated anchors must support the action?
9. What should happen on interruption, timeout, or cancellation?
10. What MuJoCo and physical evidence will establish that the acting reads?

Then use [Author and validate motion](../how-to/author-and-validate-motion.md)
and update the [catalog animation review](../reference/animation-principles-review.md)
as part of the same change.
## Voice attention

Confirmed Pi voice sessions can request restrained absolute attention turns.
The [attention brief](voice-attention.md) defines their 12-principles and ELEGNT
staging, temporary anchor, quiet hold, return timing and interruption rules.
