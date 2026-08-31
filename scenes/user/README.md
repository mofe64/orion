# Orion user scenes

Orion Studio saves user-authored and edited scene copies in this directory.
Built-in commissioned scenes remain directly under `scenes/` and are never
overwritten by Studio.

Rules:

- Every saved scene uses the existing `format_version: 1` schema.
- A scene name must be unique across the complete recursive scene library.
- Editing a built-in scene requires **Save As** with a new semantic name.
- Studio refuses to replace an existing user-scene file; save another version
  under a new name.
- Motion and pose events remain semantic references. Quintic interpolation,
  measured settling, limits, and hardware execution stay inside `oriond`.

The Rust runtime loads scene YAML recursively. Studio's authenticated Pi
gateway can publish a saved user scene to this same relative directory and ask
the running source-run daemon to reload the complete catalog. Reload is refused
while a scene is active, and any invalid pose, motion, cue, timing, schema, or
duplicate name causes the publish to roll back. Existing files are never
replaced; changed scenes must use a new name.
