# Voice attention

## Behaviour brief

Voice attention communicates one idea: Orion has heard the person and faces
their side of the conversation. Rustpotter on the Pi proposes a wake; Studio's
Qwen transcription must confirm it before attention is requested. A stereo
observation must identify left or right with confidence at least 0.75. Unknown
or uncommissioned microphone orientation produces no turn. Confirmation more
than three seconds after the direction observation was finalized also suppresses
the turn; language processing continues.

The absolute `attention_left` and `attention_right` motions use complete named
poses and the existing `expressive_turn` style. Base yaw leads a small arc;
restrained pitch, roll, shoulder compression and elbow follow support it.
Opposing anticipation, a dominant drawing and authored overshoot use `through`;
only the final conversational silhouette uses `settle`. No style, calibration,
joint convention or trajectory compiler is replaced.

This applies the 12-principles mapping in
[Character animation design](character-animation.md): pose-to-pose staging,
compression and release, anticipation, overlap, arcs, slow-in/out, secondary
action, intentional timing, restrained exaggeration, silhouette and appeal.
The [ELEGNT dimensions](expression-design.md) require small legible movement,
confirmed environmental evidence, continuous easing and calm timing. Expression
is implemented through approved capabilities, never direct motor commands.

The prior measured anchor remains immutable during the turn. Only successful
measured completion establishes the temporary conversational anchor. Speech preserves that anchor; the quiet conversational hold suppresses
autonomous idles so they cannot delay the return. After the interaction becomes neutral, a
15-second quiet interval precedes a weighted return to the previous anchor.
Explicit foreground motion, shutdown and cancellation take precedence and
discard the pending return. A failed turn never establishes a new anchor.
Attention is declined outside a bounded yaw transition or while higher-priority
work owns movement. Repeated observations do not accumulate turns.

## Acceptance

Automated validation must cover calibration, exact final targets, continuous
through drawings, cancellation, anchor ownership and startup failures. Review
both directions in MuJoCo at normal speed and check a clear, restrained arc,
balanced silhouette, head/body hierarchy and uninterrupted arrival. Physical
acceptance must additionally confirm stereo channel orientation, confidence in
the assembled enclosure, servo noise, cable clearance and readable attention
in conversation. Software checks do not establish physical acceptance.
