# Orion history storage

`orion-history` stores conversations and behavior events for the agent, coordinator, runtime and service. It runs inside those processes without a separate server.

## Storage

The default directory is `~/.local/share/orion/voice-stack/history`. `ORION_VOICE_HISTORY_DIR` selects another directory. The runtime and voice service must use the same directory.

The storage layout is:

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

Turn files contain accepted commands, replies, errors and delivery outcomes. Conversation metadata controls the conversation window and memory extraction. Optional summary files provide text context for recovery.

First use creates a format marker. It removes legacy date-named history directories without importing them. Unknown format versions stop access without resetting storage.

## Write and read behavior

An operating-system file lock serializes conversation access. JSON writes use temporary files, file synchronization and atomic replacement. Unix store directories use mode `0700`; files use `0600`.

A background writer appends behavior events through a bounded queue. Readers skip malformed journal lines. Shutdown drains queued events. Later successful writes identify gaps after disk errors.

Conversation pages contain up to 50 groups. Behavior pages contain up to 100 events. Cursors combine timestamps and IDs. The store has no automatic retention policy.

The store excludes camera pixels and audio. See [conversation policy](../docs/voice-architecture.md#agent-conversation-and-memory).

## Development

1. Run the crate tests from the repository root.

   ```bash
   cargo test --manifest-path history/Cargo.toml --locked
   ```

2. Check Rust formatting.

   ```bash
   cargo fmt --manifest-path history/Cargo.toml -- --check
   ```
