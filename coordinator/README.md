# Orion coordinator

`orion-coordinator` runs the voice pipeline inside the Pi's `orion-service`
process. It owns listener connections, wake confirmation, ASR and TTS jobs, agent
calls, response buffering, gateway uploads and playback acknowledgement.

A host supplies `CoordinatorConfig` and an `orion_agent::AgentHandle` to
`Coordinator::start`. Keeping the owning `AgentService` alive lets an idle
coordinator restart preserve the agent conversation. The Python workers in
[`speech/`](../speech/README.md) receive inference jobs over private pipes.
The coordinator calls the agent through Rust channels.

`connection()` returns the authenticated protocol-7 observer endpoint.
`events()` returns a bounded snapshot with a generation and increasing event IDs.
The Pi gateway exposes these snapshots to Studio. `set_microphone()` sends a
listener control request; `set_voice()` changes the next response's preset.
Dropping the coordinator cancels its speech run and stops both Python workers.

Microphone status checks run once per second over temporary control WebSockets.
Each successful request closes the connection, allowing up to one second for
cleanup after a ten-second request deadline. A missing close reply does not
invalidate an acknowledged mute change. The listener tolerates abrupt control
client disconnects while capture continues.

## Modules

| Module | Responsibility |
| --- | --- |
| `service.rs` | Coordinator lifecycle, observer listener and microphone controls |
| `pipeline.rs` | Voice session events, concurrent work, uploads and playback completion |
| `speech.rs` | Python worker lifecycle and inference framing |
| `buffer.rs` | Startup audio reserve and buffering for complete responses |
| `session.rs` | Wake phrase and utterance-state validation |
| `gateway.rs` | Authenticated HTTP operations and WAV construction |
| `hub.rs` | Bounded event replay and independent observers |

The listener owns capture, endpointing, the echo guard and follow-up window.
`oriond` owns hardware execution. See the [voice architecture](../docs/voice-architecture.md)
for session ordering and streaming behavior.

## Tool feedback

Search acknowledgement uses the current voice session and a separate speech run.
After playback, `session.processing` returns the listener to processing and the
runtime to thinking feedback. Final playback waits for that acknowledgement;
only the final response opens the follow-up window. The listener advertises
support through `toolFeedback`. Older peers receive final speech directly.

Lighting calls use `lamp_effect` through the gateway. The execution result returns
to the agent. Memory calls run silently inside the agent service.

## Validation

Run from the repository root:

```bash
cargo test --manifest-path coordinator/Cargo.toml
cargo clippy --manifest-path coordinator/Cargo.toml --all-targets -- -D warnings
```

Integration tests use `python3`, local sockets, and fake listener, gateway, speech
and Codex peers. They cover orchestration without model downloads or account
usage. Physical microphone, echo and playback checks require the Pi.
