# Orion service

`orion-service` hosts the Pi agent, voice coordinator, settings and profiles. The `orion-voice-stack` systemd service runs it without a desktop interface.

Studio uses the same crate as a remote client. It forwards requests through the paired Pi gateway. Closing Studio does not stop the Pi host.

## Commands

| Command | Function |
| --- | --- |
| `serve` | Run the host |
| `serve --no-autostart` | Run without starting the saved voice configuration |
| `status` | Read host status |
| `check` | Check saved settings and the installed speech package |

The host finds its release through `release.json` above its executable. An owner lock prevents duplicate hosts. SIGTERM stops workers and the agent.

## Source layout

| Module | Function |
| --- | --- |
| `lib.rs`, `main.rs` | Host lifecycle |
| `release.rs` | Release and speech environment discovery |
| `rpc.rs` | Owner lock and authenticated local control |
| `coordinator.rs`, `agent.rs` | Voice and agent lifetimes |
| `remote.rs`, `pairing.rs` | Pi connections and desktop credentials |
| `settings.rs` | Saved settings and model paths |

The `lists` request reads or changes shopping and to-do lists. Snapshot checks protect changes from concurrent edits. The `geocode` request returns location choices before a weather profile update.

## Development

1. Run the crate tests from the repository root.

   ```bash
   cargo test --manifest-path orion-service/Cargo.toml --all-targets --locked
   ```

2. Run the Rust lint checks.

   ```bash
   cargo clippy --manifest-path orion-service/Cargo.toml --all-targets --locked -- -D warnings
   ```

See [installation](../docs/quickstart.md) and [configuration](../docs/configuration.md).
