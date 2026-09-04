# Orion product vision and principles

Orion is an expressive robotic lamp designed to improve a person's working
environment through useful light, legible movement, attention, and restrained
social presence. It should behave as a dependable product before it behaves as
an open-ended robotics demonstration.

## Product priorities

When priorities conflict, use this order:

1. Human and hardware safety.
2. Reliable task lighting and predictable control.
3. Clear, recoverable interaction.
4. Quiet, legible expression.
5. Additional intelligence or novelty.

An expressive behaviour is unsuccessful if it obscures the task, surprises
the user, risks hardware, or makes failure harder to understand.

## Design principles

- **Semantic control:** higher layers request meaningful capabilities; the
  runtime owns physical execution.
- **Simulation and hardware parity:** MuJoCo and physical drivers implement the
  same contract.
- **Explicit state:** lifecycle, ownership, and failure are observable rather
  than inferred from animation or timing.
- **Local-first operation:** manual control, scenes, runtime safety, raw voice
  audio, speech recognition, and speech synthesis do not require a cloud
  service.
- **Optional provider boundary:** a configured cloud agent may receive a
  confirmed text command only after the user enables that mode. The selected
  provider and data boundary must be visible.
- **Progressive autonomy:** no generative system receives physical authority
  until a deterministic capability layer can validate, constrain, confirm,
  observe, and cancel its requests.
- **Evidence before expansion:** commission and evaluate each physical or
  interaction layer before adding another source of complexity.