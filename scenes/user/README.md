# Orion user scenes

Orion Studio saves user-authored and edited scene copies in this directory.
Built-in commissioned scenes remain directly under `scenes/` and are never
overwritten by Studio.

Rules:

- Every saved scene uses the existing `format_version: 1` schema.
- A scene name must be unique across the complete recursive scene library.
- Editing a built-in scene requires **Save As** with a new semantic name.
- **Save As** refuses to replace an existing user-scene file.
- A user scene loaded from the Pi may be updated in place only when Studio
  supplies its exact content revision. Stale revisions are rejected.
- Motion and pose events remain semantic references. Quintic interpolation,
  measured settling, limits, and hardware execution stay inside `oriond`.

The Rust runtime loads scene YAML recursively. Studio's authenticated Pi
gateway lists and reads this Pi-hosted library, can create a saved user scene,
and asks the running source-run daemon to reload the complete catalog after a
write. Reload is refused while a scene is active, and any invalid pose, motion,
cue, timing, schema, or duplicate name causes the write to roll back. Built-in
files are never replaced. Existing user files change only through a serialized,
revision-checked update; the previous bytes are restored if runtime validation
fails.
