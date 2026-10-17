# Orion

Orion is an expressive robotic lamp. Its Raspberry Pi runs the hardware runtime,
local speech recognition and synthesis, and the Codex agent coordinator. Studio
provides scene authoring, robot controls, and voice settings. Speech works with
Studio closed; Codex inference and web search still require the internet.

## Start here

1. [Quickstart](docs/quickstart.md): set up a Pi, connect Studio and deploy updates.
2. [System architecture](docs/system-architecture.md): the processes on the Pi and how they talk.
3. [How Orion moves](docs/motion-architecture.md): from a request to servo movement.
4. [Voice architecture](docs/voice-architecture.md): wake word, transcription, agent and reply.
5. [Hardware versions](docs/hardware-versions.md): V1 and V2 lamps, V2 mechanical rest zeroing.

On a freshly imaged Raspberry Pi OS Lite 64-bit Pi 5, install with:

```bash
curl -fsSL https://raw.githubusercontent.com/mofe64/orion/main/scripts/install.sh | bash
```

Then reboot, run `codex login`, assign new servo IDs, fold into mechanical rest,
and run `oriond servo zero` followed by `oriond --check` before starting Orion.

Everything else is in the [documentation index](docs/README.md).

## How a conversation flows

```text
microphone ─▶ wake word ─▶ record command ─▶ transcribe ─▶ Codex agent
                (Rustpotter)  (Silero)         (Qwen)          │
speaker + movement ◀─ oriond ◀─ gateway ◀─ synthesize reply ◀──┘
                                             (Piper)
```

Everything above runs on the Pi. Only Codex inference and web search go to the
internet. `oriond` plays the reply, animates the lamp to match the speech, and
owns the servos and the RGBW light. Studio connects to the Pi's authenticated
gateway for authoring, controls and settings; closing Studio does not stop the
lamp.

## Repository map

| Path | Responsibility |
| --- | --- |
| `agent/` | Rust conversation runtime, memory/tools, and Codex integration |
| `history/` | Shared JSON conversation storage and JSONL behaviour journal |
| `orion-service/` | Pi voice/agent host and Studio remote client |
| `runtime/` | Rust `oriond` daemon, hardware and MuJoCo backends, lifecycle, scenes, lighting, and playback |
| `coordinator/` | Reusable Rust voice orchestration, Pi transport, buffering, and playback lifecycle |
| `speech/` | Qwen speech recognition and Piper speech output on the Pi |
| `orion_studio/` | Tauri/React desktop application and Pi gateway |
| `motion/` | Pose and motion assets plus Python consumers of Rust-compiled trajectories |
| `scenes/` | Versioned multimodal scene documents |
| `description/` | Neutral Unified Robot Description Format (URDF) model and shared mesh assets |
| `simulation/mujoco/` | MuJoCo model, playback tools, and simulator checks |
| `hardware/` | Setup and operating instructions for servos, audio, and lighting |
| `voice/` | Pi microphone capture, Rustpotter, Silero endpointing, and coordinator transport |
| `audio/` | Named local audio cues |
| `docs/` | Cross-system architecture, configuration, animation references, and learning notes |

## Validation

Run these from the repository root unless a linked guide says otherwise:

```bash
cargo test --manifest-path orion-service/Cargo.toml
python3 -m unittest discover -s scripts/tests -v
cargo test --manifest-path coordinator/Cargo.toml
cargo test --manifest-path agent/Cargo.toml
cargo test --manifest-path history/Cargo.toml
cargo test --manifest-path runtime/Cargo.toml --all-targets
PYTHONPATH=motion .venv/bin/python -m pytest -q motion/test
uv run --project orion_studio --locked --managed-python python -m unittest discover -s orion_studio/tests -v

cd orion_studio
pnpm test
pnpm build
```

Some runtime integration tests expect the repository Python environment at
`.venv/bin/python`. Model-independent voice-worker tests use the worker's own
environment; see its [validation instructions](speech/README.md#validation).
Every Python component uses uv-managed Python 3.12, including speech and the
Studio gateway. Run `uv sync --locked --managed-python` in each component.
Release venvs use `/opt/orion/python`, independent of Bookworm’s Python 3.11 or
Trixie’s Python 3.13. Workstation simulation uses its own managed 3.12 venv.

## Operating limits

Speech recognition and synthesis run locally on the Pi. Codex replies and web
search require internet access. Orion waits for reply playback to finish before
accepting a follow-up; acoustic echo cancellation and interruption during playback
are not implemented.

The gateway uses HTTP with a bearer token on a trusted local network. The agent
can search, manage explicitly requested memories and change the lamp through
validated tools. Movement remains under the runtime and explicit robot controls.
See [configuration](docs/configuration.md) for stored settings and credentials.

## Safety

Before powering or moving physical hardware, follow the
[V2 mechanical rest procedure](docs/hardware-versions.md#calibrate-v2-first). Software torque
disable is not an emergency stop. Keep an accessible physical power or torque
interruption available during hardware work.
