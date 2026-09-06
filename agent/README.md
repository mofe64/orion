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

`src/codex.rs` handles the Codex JSON protocol; `src/service.rs` serializes calls
and owns cancellation; `src/lib.rs` holds public types and Orion's instructions.
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
