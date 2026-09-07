# Orion agent runtime

`orion-agent` owns Orion's Codex conversation, instructions, runtime discovery,
and spoken-response handling. It has no Tauri, audio-model, gateway, or hardware
dependencies. Studio keeps it alive independently of the voice coordinator.

`AgentService::start(AgentConfig)` starts an independent executor. Pass its
cloneable `service.handle()` to the [coordinator](../coordinator/README.md).
`AgentHandle::info()` reads status and `AgentHandle::respond(text)` returns
spoken text. Calls use bounded in-process Rust channels, not a network socket.
Dropping an active call cancels it; dropping the service shuts down Codex.

The first call starts an installed `codex app-server` process, checks its account
and model catalog, and creates an ephemeral conversation. Successful calls reuse
it. Keep the service alive when restarting the coordinator or reloading speech
models. See [conversation lifecycle](../docs/explanation/voice-architecture.md#agent-conversation-and-memory)
and [runtime discovery](../docs/reference/configuration.md#orion-studio).

The crate groups code by responsibility:

- `src/providers/`: Codex process lifecycle and App Server protocol.
- `src/runtime/`: serialized requests, cancellation, and service ownership.
- `src/prompt/`: base instructions and spoken-output normalization.
- `src/tools/`: tool schemas, validation, lighting catalog, and activity events.
- `src/memory/`: delimited Markdown storage, user edits, and keyword retrieval.
- `src/personality/`: curated traits, behavior choices, and generated `SOUL.md`.
- `src/profile.rs`: profile reads and mutations shared with Studio.
- `src/config.rs` and `src/types.rs`: configuration and public data types.
- `src/lib.rs`: stable public exports for Studio and the coordinator.

An installed Codex executable and existing login are required. There is no
Python SDK or Python agent client.

## Validation

From the repository root:

```bash
cargo test --manifest-path agent/Cargo.toml
```

Tests launch a fake App Server using `python3` and cover conversation reuse,
matching final messages, failed turns, unexpected interactive requests, and
cancellation. They do not invoke a model or use a real account. The executable
fixture is tested on macOS; Windows execution requires adaptation.

An optional installed-runtime check reads account status and the model catalog
and creates an ephemeral thread without submitting a model turn:

```bash
cargo test --manifest-path agent/Cargo.toml installed_runtime_handshake -- --ignored
```

## Memory and tools

Memory defaults to `$HOME/.local/share/orion/MEMORY.md`; `ORION_MEMORY_PATH`
selects a different file. `AgentConfig.memory_path = None` disables storage.
Entries have this format:

```markdown
<!-- memoryEntry {"id":"unique-id","created":"2026-09-06T18:30:00Z"} -->
The user prefers temperatures in Celsius.
<!-- memoryEntryEnd -->
```

The handler assigns UUIDs and UTC timestamps, rejects delimiter injection,
locks writes, and atomically replaces the document. Entries are limited to
2,000 bytes and the file to 1 MiB. Search ranks keyword matches and returns at
most eight entries; it is not semantic retrieval. Personal memories stay outside
the repository. Retrieved memories are sent to Codex when used. Studio Settings supports viewing, adding, editing, deleting, and clearing memories.
Edits compare the original entry before writing; clearing compares the complete
list, so a stale screen cannot silently erase a newer append. Automatic memory
collection is not enabled.

`respond_with_events(text, Some(sender))` reports `SearchStarted` and routes
`SetLighting` requests to a coordinator with a one-shot result channel.
`respond(text)` still works for text and memory/search tools; lighting fails
explicitly if no coordinator is attached. There is no direct hardware client
inside this crate.

Codex's native live search executes inside App Server. Orion's three dynamic
tools use the experimental `item/tool/call` protocol. The registered lighting
schema publishes moods, colors, and effects from `src/tools/lighting.rs`:
`ambient` combines warm white and amber, `cool` combines cool white and blue,
`warm` uses warm white, and `warm_red` combines warm white and red. Brightness
is absolute percent. Effect palettes default to warm white and a random accent;
explicit palettes contain one or two named colors.

Opt-in model tests use temporary memory and intercept lighting without hardware:

```bash
cargo test --manifest-path agent/Cargo.toml installed_runtime_memory_tool_roundtrip -- --ignored
cargo test --manifest-path agent/Cargo.toml installed_runtime_search_and_lighting_tools -- --ignored
```

## Personality and profile edits

Studio Settings offers five traits and four conversational habits. Defaults are
Warm, Calm, and Keep it brief. Selecting none uses a neutral, clear tone.
`SOUL.md` defaults to `$HOME/.local/share/orion/SOUL.md`; `ORION_SOUL_PATH`
overrides its location and `AgentConfig.soul_path = None` disables persistence.
The file is created on the first save. Its first HTML comment contains an
`orionSoul` JSON object with a revision and selected trait/behavior IDs, followed
by readable generated instructions. Only validated catalog IDs determine the
prompt; editing the prose does not add instructions or alter tool permissions.

`AgentHandle::profile(None)` reads the profile without starting Codex.
`profile(Some(change))` serializes user edits with agent requests. Successful
writes retire the active Codex conversation; the next request loads the saved
personality into fresh context. This also prevents deleted memories persisting
in the current thread. The coordinator keeps its handle and speech models.
Failed validation or revision conflicts preserve the existing conversation.
Deleting local memories does not erase information already sent to Codex.
