# Orion documentation

Use this guide to set up, operate, understand, and develop Orion. The library
follows a Diátaxis-style separation: tutorials teach, how-to guides complete a
task, explanations build a mental model, and references define exact contracts.
Project and decision records describe status and rationale without replacing
current technical documentation.

## Tutorials

- [Run the Orion runtime in MuJoCo](tutorials/first-runtime-run.md)
- [Run Orion Studio](tutorials/first-studio-run.md)
- [Run Studio Voice for the first time](tutorials/first-studio-voice-run.md)

## How-to guides

- [Validate character v2 on physical Orion](how-to/validate-character-v2.md)
- [Author and validate Orion motion](how-to/author-and-validate-motion.md)
- [Manage Studio Voice models](how-to/manage-studio-voice-models.md)
- [Deploy the runtime and gateway to the Pi](../runtime/README.md#deploy-an-update-to-the-raspberry-pi)
- [Commission the STS3215 servos](../hardware/servo_setup/README.md)
- [Commission the ReSpeaker audio path](../hardware/audio/README.md)
- [Commission the RGBW light](../hardware/lighting/README.md)

## Explanation

- [System architecture](explanation/system-architecture.md)
- [Motion and animation architecture](explanation/motion-and-animation-architecture.md)
- [Character animation design](explanation/character-animation.md)
- [Voice architecture](explanation/voice-architecture.md)
- [Product vision and principles](explanation/product-vision.md)
- [ELEGNT expression model](explanation/expression-design.md)

## Reference

- [Motion asset schemas and catalog](reference/motion-assets.md)
- [Trajectory and joint-control internals](reference/trajectory-and-joint-control.md)
- [Animation-principles catalog review](reference/animation-principles-review.md)
- [Platform support](reference/platform-support.md)
- [Configuration and environment variables](reference/configuration.md)
- [Licensing and source provenance](reference/licensing-and-provenance.md)
- [Runtime command and lifecycle reference](../runtime/README.md)
- [Scene format and lifecycle](../scenes/README.md)
- [Motion asset rules](../motion/README.md)
- [Local audio cue rules](../audio/README.md)
- [Robot description](../description/README.md)

## Project information

- [Implementation status](project/status.md)
- [Roadmap](project/roadmap.md)
- [Known limitations](project/known-limitations.md) — important operational and
  product constraints.
- [Architecture decisions](decisions/README.md)

## Learning notes

- [Joint structure](learning_notes/orion_joints.md)
- [MuJoCo model](learning_notes/orion_mujoco_model_basics.md)
- [URDF basics](learning_notes/orion_urdf_basics.md)

## Documentation ownership

Documentation changes ship with the code or asset change they describe. Use
these ownership rules to prevent contradictory explanations:

- A concept has one canonical explanation and other documents link to it.
- Exact fields, constants, states, and invariants belong in reference docs.
- Operational sequences belong in how-to guides or the owning component
  README.
- Cross-component reasoning belongs in `docs/explanation/`.
- Durable architectural choices belong in an accepted decision record.
- Status, roadmap, and limitations describe project state; they are not API or
  runtime contracts.
- Package READMEs define local ownership and entry points, then link to
  canonical cross-system documentation.

New canonical documents should state their audience, scope, source of truth,
and what they deliberately defer to. When implementation and documentation
disagree, code and validated configuration are authoritative until both are
corrected in the same change.

Run the repository documentation gate before merging:

```bash
python3 scripts/check_docs.py
```

The gate checks every tracked and newly added Markdown file for one level-one
heading, valid local links including section anchors, and prohibited
motion-system terminology that would reintroduce superseded documentation.
