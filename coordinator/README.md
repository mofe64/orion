# Orion coordinator

`orion-coordinator` owns the voice pipeline: Pi connections, wake confirmation,
follow-up turns, ASR jobs, agent calls, TTS jobs, startup buffering, uploads,
playback acknowledgement, tool-progress speech, lighting tool execution, and cancellation. It has no dependency on Tauri.

Studio includes this library through a Cargo path dependency. A launcher supplies
`CoordinatorConfig` and an `orion_agent::AgentHandle` to `Coordinator::start`.
Keep the owning `AgentService` alive independently so idle coordinator restarts
preserve conversations. `connection()` returns the authenticated protocol-7
observer endpoint; `set_microphone()` sends an explicit Pi control request.
Dropping the coordinator cancels its owned speech run and stops its Python child.

The Python worker lives in [`speech/`](../speech/README.md) and receives only
inference jobs over private pipes. The Rust agent handle is called directly;
there is no Python agent client or agent TCP bridge. Studio remains the supplied
launcher. A standalone headless launcher is not implemented.

## Modules

| Module | Responsibility |
| --- | --- |
| `service.rs` | Application lifecycle, observer listener, microphone control |
| `pipeline.rs` | Pi session events, sequential stages, uploads and completion |
| `speech.rs` | Python child lifecycle and bounded inference protocol |
| `buffer.rs` | Conservative startup buffering policy |
| `session.rs` | Wake phrase and utterance-state validation |
| `gateway.rs` | Authenticated HTTP operations and WAV construction |
| `hub.rs` | Bounded status replay and independent UI observers |

The Pi retains microphone capture, wake detection, echo guard, and the five-second
listening window. `oriond` retains hardware execution. See the
[voice architecture](../docs/explanation/voice-architecture.md) for state and
transport contracts.

## Validation

Run from the repository root:

```bash
cargo test --manifest-path coordinator/Cargo.toml
cargo clippy --manifest-path coordinator/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path orion_studio/src-tauri/Cargo.toml
```

Integration tests require `python3` and local socket access. Fake Pi, gateway,
speech, and Codex peers exercise the production coordinator without hardware,
model downloads, or account usage. Python inference remains Apple-Silicon-only;
physical microphone, echo, and playback acceptance requires a real Pi.

## Tool feedback compatibility

The listener advertises `toolFeedback: true` when it accepts a transition from
intermediate playback back to processing. Search acknowledgement uses the same
voice session and a separate speech run; it does not finish the session or open
a conversation window. Final playback waits for the acknowledgement. Upgrade
the Pi listener and runtime together for breathing/animation after intermediate
speech. Older listeners receive the final answer without the acknowledgement.

Lighting calls use `lamp_effect` through the gateway and require the updated Pi
gateway and runtime. Failed execution is returned to the agent. Memory calls
execute silently in the agent service.
