# Validate Orion character v2 on physical hardware

Run this focused acceptance pass after the atomic v2 deployment succeeds. It
verifies the character behavior that simulation cannot prove: readable motion
under load, holding drift, interruption quality, ReSpeaker playback, and the
final mechanical release. It does not replace servo commissioning.

## Safety and evidence

Use a clear bench with Orion's base supported, its full movement envelope free
of tools and cables, and a physical power or torque interruption within reach.
Only one operator should issue commands. Stop immediately for unexpected
contact, cable tension, harsh servo noise, loss of control, rising temperature,
or movement toward a mechanical limit.

Record the deployed revision and calibration before moving Orion:

```bash
cd /home/mofe/dev/orion
git rev-parse --short=12 HEAD
sha256sum /home/mofe/.config/orion/servo_calibration.json
runtime/target/release/oriond --status
```

The initial status must report torque disabled and character mode off. Keep a
screen recording of Studio diagnostics or save status JSON at each numbered
gate. Accept a result only when the observed behavior and terminal state both
match the expectations below.

## 1. Start and expressive arcs

Connect Studio to the Pi gateway and choose **Start character**. Wait until
diagnostics report `home_idle`, `home` is held, and no foreground clip is
active.

Run `acknowledge_left`, then `acknowledge_right`. For each scene verify:

- anticipation, lean, and authored overshoot read as one uninterrupted arc;
- there is no visible stop at an internal drawing;
- the light pulse and warm cue coincide with the `notice` marker;
- the terminal scene state is `completed`; and
- the active idle anchor changes to the measured final pose.

Any jerk, zero-velocity pause, marker desynchronization, timeout, or unexpected
limit approach fails this gate.

## 2. Held anchors and autonomous idle

Exercise `home`, `attentive`, `look_left`, and `look_right` as separate held
anchors. `return_home` establishes `home`; `attentive_entry` establishes
`attentive`; the two acknowledgement scenes establish the directional anchors.

At each anchor, leave character mode undisturbed long enough to see at least
two micro-idles and one larger idle. Because timers are deliberately jittered,
allow up to 90 seconds after the preceding action for each observation window.
Verify that:

- timings are visibly non-periodic and the same clip is not chosen twice in a
  row;
- each idle starts from and returns to the immutable anchor;
- the five measured joint positions settle within `0.05 rad` of that anchor;
- the next idle starts from the same anchor rather than the preceding idle's
  endpoint; and
- there is no cumulative drift after the complete observation window.

Routine idles must not play an audio cue. Directional and attentive anchors
must select compatible idle profiles.

## 3. Foreground interruption

Wait until Studio diagnostics shows a non-empty idle `active_clip`, then
immediately run `curiosity` or `acknowledge_left`. Verify that the foreground
scene begins from the measured in-flight state without first snapping back to
the old anchor. The idle must become cancelled, the scene must complete, and
its measured final position must become the idle anchor.

## 4. Studio Voice through Orion

Use Studio Voice with Chatterbox selected and Pi playback visible. Produce
three responses:

1. a short one-sentence response;
2. a medium response of roughly three to five sentences; and
3. a 20–30 second self-introduction in Orion's own voice, covering who Orion
   is, their calm-curious character, and how motion, light, and speech work
   together. Longer uploads may separately exercise the 120-second contract.

For every response confirm that Studio shows Pi `queued`/`playing`/`completed`
states and does not play the response through the workstation speaker. Listen
for clear ReSpeaker output while observing:

- one utterance-length performance spline assembled from all four speaking
  drawings without immediate repetition;
- continuous movement on both neighbouring 50 Hz samples around every
  internal drawing, with no unplanned stopped plateau;
- a head-led hierarchy on every phrase: tilt, pitch, and calibrated turns must
  read before the supporting shoulder and elbow action;
- clearly readable but restrained and varied shoulder/elbow follow-through
  during ordinary speech, with no obvious fixed-cycle repetition;
- larger explanatory body beats only near selected audible phrase-energy
  peaks, never on adjacent phrases and no more often than roughly one in three;
- overlapping head and body follow-through that prevents all joints from
  stopping together;
- spatial, smoothed `speaking_energy` light response that remains secondary to motion; and
- one intentional final slow-out and smooth return to the pre-speech anchor
  before a freshly randomized idle delay begins.

Motion failure must not interrupt audible speech. An audio failure must stop
speaking animation, restore the anchor, report `failed`, and remove the spool
file.

## 5. Cancellation

Start the long Studio Voice response again. While Pi playback is active, run:

```bash
runtime/target/release/oriond --stop-speech
runtime/target/release/oriond --speech-status
```

The speech run must become `cancelled`, audio must stop, Orion must settle to
the pre-speech anchor, and its temporary WAV must be absent from
`/tmp/orion-speech-spool`.

Then start a foreground scene and cancel it before completion:

```bash
runtime/target/release/oriond --run-scene thinking
runtime/target/release/oriond --stop-scene
runtime/target/release/oriond --scene-status
```

The scene must become `cancelled`; scene-owned motion, lighting transition, and
audio must stop without leaving an active run.

## 6. Home, mechanical rest, and torque release

Choose **Stop character** in Studio. Wait for `return_home` to complete and for
character status to report `off`. Character stop deliberately leaves powered
holding torque on at `home`; mechanical release is a separate explicit step.

Complete the shutdown from the Pi:

```bash
runtime/target/release/oriond --goto rest --duration 3.0 --wait
runtime/target/release/oriond --disable
runtime/target/release/oriond --status
```

Accept the final gate only when the `rest` movement completed, the reported
mode is `configured`, torque is disabled, character mode is off, lights are
off, and no motion, scene, or speech run is active. If rest cannot be confirmed,
do not disable torque automatically; support Orion and investigate the failed
run first.

## Acceptance record

Record pass/fail, the relevant run IDs, and concise observations for these six
gates: expressive arcs, held-anchor idles, foreground interruption, three
speech lengths, both cancellations, and final release. Attach the deployed
revision, calibration hash, and any failure status JSON. Physical character v2
acceptance is complete only when every gate passes on the assembled Orion.
