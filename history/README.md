# Orion history storage

`orion-history` shares conversation files and behaviour journals between the
agent, coordinator, motor runtime and Studio service. It runs inside those
processes and requires no database, server or migration framework. Cross-system
policy is in [conversation and memory](../docs/voice-architecture.md#agent-conversation-and-memory).

## Files and configuration

`ORION_ONBOARD=1` enables `~/.local/share/orion/voice-stack/history` by default.
`ORION_VOICE_HISTORY_DIR` overrides it and enables storage on other hosts.
Set the same directory for `oriond` and the voice service.

```text
history/
  format.json
  .lock
  conversations/<conversation-id>/
    conversation.json
    turns/<session-id>.json
    extraction.json
    context-summary.json
  behaviours/YYYY-MM-DD.jsonl
```

`extraction.json` and `context-summary.json` exist only when needed. Conversation
metadata defines the fixed window and extraction state. Turn files hold accepted
command text, reply, errors, generation/delivery outcomes and dated activity.
Summaries contain text-only recovery context, not long-term memory. Automatic
memory receipts live beside `MEMORY.md` in `MEMORY.automatic.json`.

First use creates a version marker and deletes old `YYYY-MM-DD` history
directories without reading or importing their turns. Other files, including
memory, personality and settings, are preserved. Unknown format versions fail
without resetting storage.

## Reliability and limits

A shared OS file lock serializes conversation reads and writes. JSON writes use
a synced temporary file, atomic replacement and directory sync. Directories use
mode `0700` at the store root, files `0600` on Unix. Behaviour writers append and sync on a background
thread using a bounded queue; they separate a crashed partial tail from later
records. Readers skip malformed journal lines. Shutdown drains queued events.
Disk errors are reported and subsequent successful writes record gaps.

Conversations page 50 groups at a time; behaviours page 100 events, optionally
filtered by category. Timestamp-plus-ID cursors disambiguate ties. Reads scan
files; there is no index or automatic retention policy. Camera pixels and audio
are excluded. The journal records actions and observations, not persistent
recognition of people or objects.

## Validation

Run from the repository root:

```bash
cargo test --manifest-path history/Cargo.toml --locked
cargo fmt --manifest-path history/Cargo.toml --check
```

Tests cover false wakes, exact expiry, restart and late results, retirement
readiness, context cutoffs, legacy cleanup, unknown formats, journal crash tails,
permissions, filtering and pagination.
