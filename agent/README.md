# Orion agent runtime

`orion-agent` owns Codex conversations, instructions, tools and spoken responses. It has no direct hardware client. The service host keeps it alive independently of speech workers.

## Host interface

`AgentService::start` creates an executor. Its cloneable handle provides status, response and profile requests through bounded Rust channels. Dropping an active call cancels it. Dropping the service stops Codex.

The first status or model request starts `codex app-server`. Orion checks account access and the model catalogue before creating conversation context. An installed Codex executable and login are required.

Completed turns can reuse provider context. Uncertain protocol state retires that context. A replacement provider receives bounded text recovery context. See [conversation policy](../docs/voice-architecture.md#agent-conversation-and-memory).

## Source layout

| Path | Function |
| --- | --- |
| `src/providers/` | Codex process and protocol |
| `src/runtime/` | Requests, cancellation and ownership |
| `src/context.rs` | Replacement-provider context |
| `src/extraction.rs` | Conversation retirement proposals |
| `src/prompt/` | Instructions and spoken text |
| `src/tools/` | Checked tool inputs and results |
| `src/memory/` | Memory storage and retrieval |
| `src/personality/` | Personality catalogue and generated instructions |
| `src/profile.rs` | Profile access shared with Studio |

## Tools and hardware boundary

Robot tools send typed requests to the coordinator. The coordinator returns the runtime's result. Tools cannot send raw joint goals. Calls after final speech begins cannot start hardware operations.

`look` requests one image for an explicit visual question. `aim_light` requests a supported desk-object lookup and runtime aim. Results distinguish `aimed`, `clamped`, `not_found` and `ambiguous`.

Lighting tools expose named effects, colors and brightness. Relative changes first read the manual lamp state. Scenes, speech and rest retain output ownership.

Timer, alarm, reminder and mode tools use the runtime scheduler. Weather uses fixed Open-Meteo endpoints. List and memory tools use local stores.

## Memory and profile storage

The [configuration reference](../docs/configuration.md#saved-files) defines paths. Each memory entry has an ID, timestamp and bounded text. The memory file has a 1 MiB limit; each entry has a 2000-byte limit.

Writes use a file lock and atomic replacement. Updates compare the original entry. Clearing compares the complete list. This prevents an old view from silently replacing newer data.

Search uses normalized whole words and returns at most eight entries. Retrieved memories enter Codex context when used. Deleting local data does not remove information already sent to Codex.

`MEMORY.automatic.json` stores extraction receipts and explicit-edit cutoffs. Preserve it with the memory file. It protects against duplicate extraction and reintroducing forgotten entries.

Personality choices generate `SOUL.md`. Catalogue IDs determine instructions; editing generated prose does not change tool permissions. Successful profile changes replace provider context while retaining the logical conversation group.

## Lists and reminders

Shopping and to-do lists each accept up to 200 items. Item text contains 1–200 printable characters. Removal checks the original item. Clearing requires confirmation and an unchanged list snapshot.

Next-conversation reminders remain pending until completed playback acknowledges their delivery. Failed or interrupted replies retain them. Timed reminders use the runtime scheduler.

## Development

1. Run the crate tests from the repository root.

   ```bash
   cargo test --manifest-path agent/Cargo.toml --locked
   ```

2. Run the optional account handshake with an installed, signed-in Codex executable.

   ```bash
   cargo test --manifest-path agent/Cargo.toml installed_runtime_handshake -- --ignored
   ```

The handshake reads account and model information without submitting a model turn. Other ignored model tests can use account inference.
