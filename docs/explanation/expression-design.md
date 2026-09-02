# ELEGNT expression model

ELEGNT is Orion's framework for designing expression that supports rather than
competes with the lamp's task.

## Behaviour dimensions

- **Ease:** motion should begin, travel, and settle without mechanical or
  perceptual harshness.
- **Legibility:** the user should be able to read the intended target, state,
  or acknowledgement.
- **Economy:** use the smallest movement, light change, or sound that
  communicates successfully.
- **Grounding:** expression should follow real runtime and environmental state,
  not decorative randomness.
- **Natural timing:** pauses, overlap, and response timing should feel
  intentional while remaining deterministic enough to test.
- **Temperament:** Orion should feel calm, attentive, and competent rather than
  hyperactive or theatrical.

## Behaviour design template

Before implementing an expressive behaviour, record:

1. The user-facing purpose.
2. The observable trigger and required confidence.
3. The named pose, motion, light, audio, or scene capabilities it may use.
4. Its start, active, settling, interrupted, failed, and completed states.
5. The safety and task-clarity constraints.
6. How the user can interrupt or recover from it.
7. What evidence would show that the behaviour is useful and legible.

Expression belongs above the validated capability boundary. It may choose or
modulate approved semantic behaviour, but it must not own servos, GPIO, audio
devices, calibration, or safety limits.

See [Character animation design](character-animation.md) for the concrete
motion language, 12-principles mapping, idle behavior, and speech hierarchy
that implement these dimensions.
