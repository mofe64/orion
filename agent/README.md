# Orion agent runtime

`orion-agent` owns Orion's Codex conversation, instructions, runtime discovery,
and spoken-response handling. It has no Tauri, audio-model, gateway, or hardware
dependencies. The [Orion service](../orion-service/README.md) keeps it alive
independently of the voice coordinator on the Pi.

`AgentService::start(AgentConfig)` starts an independent executor. Pass its
cloneable `service.handle()` to the [coordinator](../coordinator/README.md).
`AgentHandle::info()` reads status and `AgentHandle::respond(text)` returns
spoken text. Calls use bounded Rust channels within the host process. Memory
edits explicitly unlock at completion so a descriptor inherited during a concurrent
subprocess launch cannot retain a finished write lock.
Dropping an active call cancels it; dropping the service shuts down Codex.

The first status or model-handled response request starts `codex app-server`, checks its account
and model catalog, and creates an ephemeral conversation. Calls reuse it after a matching terminal
turn event, including a completed turn with tool errors or empty speech. Protocol
uncertainty (interruption, unreadable events, stale calls, inconsistent final
speech or unsupported server requests) retires it. Keep the service alive when restarting the coordinator or reloading speech
models. See [conversation lifecycle](../docs/voice-architecture.md#agent-conversation-and-memory)
and [runtime configuration](../docs/configuration.md#pi-voice-profile).

The crate groups code by responsibility:

- `src/providers/`: Codex process lifecycle and App Server protocol.
- `src/runtime/`: serialized requests, cancellation, and service ownership.
- `src/prompt/`: base instructions, final-sentence streaming and spoken-output normalization.
- `src/tools/`: tool schemas, validation, lighting catalog, and activity events.
- `src/memory/`: delimited Markdown storage, user edits, and keyword retrieval.
- `src/personality/`: curated traits, behavior choices, and generated `SOUL.md`.
- `src/profile.rs`: profile reads and mutations shared with Studio.
- `src/config.rs` and `src/types.rs`: configuration and public data types.
- `src/lib.rs`: public exports for the host and coordinator.

An installed Codex executable and existing login are required. Follow the
[Pi login procedure](../docs/quickstart.md#pi-local-voice-and-agent) before starting
the host.

## Validation

From the repository root:

```bash
cargo test --manifest-path agent/Cargo.toml
```

Tests launch a fake App Server using `python3` and cover conversation reuse,
matching final messages, failed turns, unexpected interactive requests, and
cancellation. They do not invoke a model or use a real account. The executable
fixture uses Unix process behavior; Windows execution requires adaptation.

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
most eight entries. Personal memories stay outside
the repository. Retrieved memories are sent to Codex when used. Studio Settings supports viewing, adding, editing, deleting, and clearing memories.
Edits compare the original entry before writing; clearing compares the complete
list, so a stale screen cannot silently erase a newer append. Automatic memory
collection is not enabled.

`respond_with_events(text, Some(sender))` reports `SearchStarted` and routes
`SetLighting` and `RobotOperation` requests to a coordinator with a one-shot
result channel.
Clear immediate sleep phrases use the same validated `go_to_sleep` tool from the
agent executor before returning spoken acknowledgement. The model cannot merely
promise rest for these phrases. A rejected sleep request gets an honest failure
reply; longer or conditional requests still go to the model.
`respond(text)` still works for text and memory/search tools; lighting fails
explicitly if no coordinator is attached, as do mode, sleep and alert tools. There
is no direct hardware client inside this crate.

Codex's native live search executes inside App Server. Orion's dynamic
tools use the experimental `item/tool/call` protocol. The registered lighting
schema publishes moods, colors, and effects from `src/tools/lighting.rs`:
`ambient`, `amber`, and `orange` use the same deep golden bulb-style color:
RGBW `(255, 100, 0, 80)`. This mix includes the dedicated white LED and requires
visual tuning on the physical lamp; it is not a calibrated color temperature.
`ambient` selects one color so animated effects retain that golden hue.
`cool` combines cool white and blue,
`warm` uses warm white, and `warm_red` combines warm white and red. Brightness
is absolute percent. `get_lighting` returns the manual lamp's brightness in
percent, effect and RGBW colors; a null lamp means no manual override. Read it
before a relative adjustment. Changes are refused during a scene or speech, and
starting the character clears the manual light. Effect palettes default to warm
white and a random accent with unique RGBW values; amber and orange remain
aliases for explicit palettes.

The mode and alert tools use `src/tools/routines.rs`:

| Tool | Request |
| --- | --- |
| `set_mode` | `mode`: `idle` or `lamp` |
| `go_to_sleep` | No arguments; rest after the spoken reply |
| `set_timer` | `seconds`: 1–604800; `label`: up to 80 characters |
| `set_alarm` | `at`: future RFC3339 timestamp with UTC offset; `label` |
| `list_alerts` | No arguments; returns Pi local time, active and recent alerts |
| `cancel_alert` | `id` from a previous tool result |
| `stop_alert` | No arguments; silences alerts ringing now |

Every turn includes UTC time, local time with its offset, and a discovered IANA
zone. POSIX `TZ` settings fall through to system timezone files. When discovery
fails, the context and `list_alerts` say `unknown; use the local UTC offset above`.
For a future alarm date, clarify an offset that cannot be determined. A speech
sanity-limit failure is included in the next model turn without starting extra
inference after playback fails.

Duplicate calls or calls beyond the per-turn budget return a tool error without
repeating the action or resetting a completed conversation.

Only explicit `final_answer` items participate in multiple-final and streamed-item checks. Without an explicit final, the last
phase-less message wins; those legacy messages are never streamed early.

Read `list_alerts` before choosing a clock-alarm timestamp. Clarify ambiguous
times and cancellation targets. Clock alarms must fall within the next 366 days;
recurring alarms are unsupported. Runtime validation returns errors to the agent,
which confirms a change only after success. See
[timer and alarm behavior](../docs/system-architecture.md#timers-and-alarms).

Opt-in model tests use temporary memory and intercept lighting without hardware:

```bash
cargo test --manifest-path agent/Cargo.toml installed_runtime_memory_tool_roundtrip -- --ignored
cargo test --manifest-path agent/Cargo.toml installed_runtime_search_and_lighting_tools -- --ignored
```

## Personality and profile edits

Studio Settings offers five traits and four conversational habits. Defaults are
Warm, Calm, and Keep it brief. This habit is the only overall brevity control;
deselecting it allows long answers. The base prompt asks for a short opening
sentence and speech-friendly prose without markdown, bullet symbols, tables or
headings. Speech normalization strips citations and whitespace without truncation;
the streamed-answer runaway guard remains 64 KiB. Selecting none uses a neutral,
clear tone.
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
