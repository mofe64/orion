# Orion service

`orion-service` hosts the Pi agent, voice coordinator, saved settings and profiles.
The Pi's `orion-voice-stack` systemd unit runs the executable without a UI.
The same crate supplies Studio's `Backend`, a client that stores desktop pairing
and forwards requests to the Pi gateway.

Use the [Pi quickstart](../docs/quickstart.md#pi-local-voice-and-agent) for setup.
`ORION_ONBOARD=1` starts saved settings against the loopback listener and gateway.
The listener grants processing ownership to the local coordinator. Studio
observes events and changes settings through the gateway. The host continues
running when the desktop disconnects.

The executable accepts `serve`, `status`, and `check`. `serve --no-autostart` is
available for isolated integration tests. `check` validates the source root,
Python executable and saved settings without starting inference. SIGTERM shuts
down workers and the agent cleanly. An OS owner lock prevents duplicate hosts.
The historical discovery directory is retained for installed Pi gateways; see
[configuration](../docs/configuration.md#pi-service-control).

## Modules

| Module | Responsibility |
| --- | --- |
| `lib.rs` | Pi host lifecycle and desktop remote client |
| `main.rs` | Service startup, retry and signal handling |
| `rpc.rs` | Owner lock and authenticated loopback control |
| `coordinator.rs` | Coordinator configuration and reuse |
| `agent.rs` | Agent lifetime independent of speech reloads |
| `remote.rs` | Paired Pi discovery and bounded gateway requests |
| `pairing.rs` | Desktop credential storage |
| `settings.rs` | Saved Pi preferences and model paths |

## Validation

Run from the repository root:

```bash
cargo test --manifest-path orion-service/Cargo.toml --all-targets
cargo clippy --manifest-path orion-service/Cargo.toml --all-targets -- -D warnings
python3 -m unittest discover -s scripts/tests -v
```

Tests exercise a real service process with fake speech, agent and gateway peers.
They cover authentication, duplicate-owner rejection, complete voice turns,
shutdown cancellation and desktop errors when the Pi is unavailable. Native
keychain testing is opt-in and ignored by default.
