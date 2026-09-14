# Orion Studio service

`orion-studio-service` owns one agent and voice coordinator, plus pairing,
settings, and profile commands. The desktop uses `Backend` to attach to an
installed headless owner or create an embedded `Host`. The standalone
`orion-studio-headless` binary runs that host without Tauri or a webview.

Use the [quickstart](../docs/quickstart.md) for installation and updates. Voice
inference requires Apple Silicon. The management scripts use macOS launchd;
other platforms require separate service integration and validation.

## Direct development

Prepare the [speech worker](../speech/README.md#setup-on-apple-silicon), save
pairing in Studio, quit its embedded session, and run from the repository root:

```bash
cargo run --manifest-path studio-service/Cargo.toml -- serve
```

`status` reads the active process status. `check` validates source paths, the
Python executable, and saved settings without starting voice. `serve --no-autostart`
starts only the control service until a client explicitly requests voice; it is
used by isolated integration tests. Use Ctrl-C or SIGTERM for graceful shutdown.

The owner lock covers the entire host lifetime. Never delete `owner.lock` to
force startup. A stale connection file can remain after a crash; starting the
headless process replaces it after acquiring the lock. An installed but stopped
service causes desktop requests to report that it needs to be started.

## Modules

| Module | Responsibility |
| --- | --- |
| `lib.rs` | Host lifecycle, commands, and desktop attachment |
| `main.rs` | Standalone listener, startup retry, and signal handling |
| `rpc.rs` | Owner lock, private discovery file, authentication, bounded requests |
| `coordinator.rs` | Coordinator configuration and reuse |
| `agent.rs` | Agent lifetime independent of speech reloads |
| `pairing.rs` | Native credential storage |
| `settings.rs` | Saved preferences, validation, model and cache paths |

The local control protocol uses one authenticated JSON request per TCP connection.
It has a 1 MiB input limit, a five-second read deadline, a 130-second dispatch
deadline, and at most 16 concurrent handlers. The coordinator's existing observer
WebSocket remains separate. See [service lifecycle](../docs/voice-architecture.md#studio-service-lifecycle)
for the cross-system ownership rules.

## Validation

Run from the repository root:

```bash
cargo test --manifest-path studio-service/Cargo.toml --all-targets
cargo clippy --manifest-path studio-service/Cargo.toml --all-targets -- -D warnings
python3 -m unittest discover -s scripts/tests -p test_studio_service.py -v
```

Integration tests launch the actual binary with fake Pi, gateway, speech, and
Codex peers. They verify voice without a UI, observer detachment, duplicate-owner
rejection, authentication, stopped-service behavior, and cancellation on shutdown.
They require `python3` and local socket access, and do not invoke a real account
or robot. The native credential test is explicitly ignored by default.

To verify launchd in a macOS login session, build the debug binary and opt in:

```bash
cargo build --manifest-path studio-service/Cargo.toml
ORION_TEST_LAUNCHD=1 python3 -m unittest discover \
  -s scripts/tests -p test_studio_service.py -v
```

This creates a temporary job with a unique label, checks crash restart and
stop/start, and removes the job. It disables voice autostart. Release rollback
tests inject activation failures and verify restoration of the previous release.
