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
listener control request.
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
| `buffer.rs` | Startup reserve and complete-reply buffering |
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
to the agent, including the runtime's exact validation reason. HTTP errors use
`error.message` with a status fallback; model-visible failures omit URLs and bearer
tokens. `get_lighting` reads the manual state through `lamp_status`. Mode and alert tools use the same route through the `routines`
operation. Sleep attaches the current voice session and suppresses the follow-up
window after acknowledgement. An alarm interruption cancels the active voice job;
the listener handles dismissal locally. Memory calls run silently inside the agent
service.

## Long replies

Text delivery queues independently of synthesis under the agent's streamed-answer
and event size guards, so slow TTS does not consume the agent generation deadline.
PCM queues remain bounded. Speech text is split at sentences or word boundaries into at most 160 Unicode
characters per TTS job, with character splits for unbroken text. Each job retains
its 240-second inference deadline. Runtime streams have a 30-minute audio sanity cap; rejection is explicit and
its reason reaches the next model turn. Playback fails after 20 seconds without a state change, accepted chunk or
advancing software playback position. Waiting for synthesis after the buffer drains
does not use this stall budget; runtime underrun and upload validation still apply.
The coordinator sends `session.keepalive` every five seconds during synthesis
and playback to extend listener/runtime deadlines without changing feedback. A hung owner still expires after its last renewal.

Fast synthesis releases a six-to-twelve-second startup reserve. Once six seconds
are available, generation taking more than 75% of audio duration, or a required
reserve above twelve seconds, latches complete buffering. Every remaining chunk
stays in the coordinator until the final end marker; later fast chunks cannot
clear the latch.

The coordinator checks the thirty-minute audio cap before retaining each chunk.
Held PCM costs 2.88 MB per minute, up to 86.4 MB for a complete buffered reply,
plus bounded in-flight chunks and allocation overhead. An oversized reply fails
with the same explicit runtime sanity-limit reason and supplies that reason to
the next model turn; it is never truncated.

Uploads wait while the runtime reports more than sixteen seconds ahead of
playback. Complete buffered replies continue uploading while the runtime is
queued, allowing their end marker to arrive before upload-idle expiry. They finish
that burst even if playback starts midway with a large buffered lead. Replies
uploaded to an already-playing runtime retain normal backpressure. A later synthesis slowdown can still
exhaust a reserve that was released on the fast path. See
[streaming replies](../docs/voice-architecture.md#streaming-replies-and-timing)
for transport ordering and progress-estimate limitations.

## Validation

Run from the repository root:

```bash
cargo test --manifest-path coordinator/Cargo.toml
cargo clippy --manifest-path coordinator/Cargo.toml --all-targets -- -D warnings
```

Integration tests use `python3`, local sockets, and fake listener, gateway, speech
and Codex peers. They cover orchestration without model downloads or account
usage. Physical microphone, echo and playback checks require the Pi.
