# Licensing and source provenance

Orion contains original implementation work and assets derived from or
informed by upstream projects. Provenance must remain explicit before the
repository, binaries, models, or physical design files are distributed.

## Repository licence state

`runtime/Cargo.toml` declares the Rust runtime as `GPL-3.0-only`, and the servo
commissioning tools use the same licence. The repository has no root licence
file. Add one before treating the whole repository's distribution terms as
fully specified.

Do not infer that every file automatically has the runtime crate's declared
licence. Add the appropriate root and third-party notices after a dedicated
licensing review.

## LeLamp-derived reference material

The mechanical reference directories under `hardware/reference_lelamp/` and
parts of the robot-description asset lineage originate from the LeLamp project.
The servo commissioning workflow also records behaviour intentionally matched
to or changed from LeLamp. Its [sources and provenance section](../../hardware/servo_setup/README.md#sources-and-provenance)
links to the exact upstream procedures and runtime files used.

Keep upstream project names, source URLs, and deliberate deviations when
transforming those assets. Do not replace provenance with a generic statement
that the files are “based on another lamp.”

## Third-party software and models

Dependency manifests pin or constrain the software used by each component, but
a manifest is not a complete distributable notice bundle. Studio Voice model
weights are downloaded from separate Hugging Face repositories and are not
stored in Git. Review each model repository's licence and usage terms before
shipping cached weights or a bundled installer.

The same rule applies to audio cues, fonts, generated assets, and evaluation
datasets: runtime availability does not prove redistribution permission.

## Release gate

Before a public or commercial release:

1. Add a root licence that accurately covers Orion-owned material.
2. Inventory third-party code, models, audio, CAD, meshes, and datasets.
3. Record source, version, licence, modifications, and required notices.
4. Confirm whether derived mechanical and software assets impose reciprocal
   distribution obligations.
5. Generate and review the notice bundle shipped with each binary or asset
   package.

This is not legal advice.
