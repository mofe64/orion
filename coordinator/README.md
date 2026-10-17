# Orion coordinator

`orion-coordinator` runs the voice pipeline inside `orion-service`. It owns listener connections, speech jobs, agent requests, gateway uploads and playback acknowledgements.

## Host interface

The host supplies `CoordinatorConfig` and an `AgentHandle` to `Coordinator::start`. Keeping `AgentService` alive preserves conversation state across coordinator restarts.

`connection()` returns the authenticated observer endpoint. `events()` returns bounded event snapshots with increasing IDs. `set_microphone()` sends a listener control request.

Dropping the coordinator cancels its speech run and stops both Python workers. The listener owns capture and speech boundaries. The runtime owns hardware execution.

## Modules

| Module | Function |
| --- | --- |
| `service.rs` | Coordinator lifecycle and observer endpoint |
| `pipeline.rs` | Sessions, concurrent jobs, uploads and playback |
| `speech.rs` | Python workers and inference messages |
| `buffer.rs` | Startup reserve and complete-reply buffering |
| `session.rs` | Wake phrase and utterance checks |
| `gateway.rs` | Authenticated HTTP requests and WAV data |
| `hub.rs` | Bounded event replay |
| `history.rs` | Accepted conversation turns |

## Reply and tool behavior

Speech text splits at sentence or word boundaries into jobs of at most 160 Unicode characters. Each job keeps its inference deadline. PCM queues remain bounded.

Slow synthesis selects complete buffering for the remaining reply. Fast synthesis uses a startup reserve. Uploads apply backpressure when playback has sufficient queued audio. Backpressure pauses a producer until its consumer has capacity.

Tool acknowledgement uses a separate speech run within the active voice session. Final playback waits for that run. Only the final reply opens the follow-up window.

Robot tool failures return the runtime's reason to the agent. Error messages omit bearer tokens. Memory and list tools run inside the agent store.

Scheduled announcements wait for idle voice ownership. Wake requests can interrupt them. Playback completion precedes acknowledgement, so acknowledgement retries cannot repeat delivered speech.

The [voice architecture](../docs/voice-architecture.md) defines session ordering and [speech workers](../speech/README.md) define inference messages.

## Development

1. Run the crate tests from the repository root.

   ```bash
   cargo test --manifest-path coordinator/Cargo.toml --locked
   ```

2. Run the Rust lint checks.

   ```bash
   cargo clippy --manifest-path coordinator/Cargo.toml --all-targets --locked -- -D warnings
   ```
