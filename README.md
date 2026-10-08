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
5. [Hardware versions](docs/hardware-versions.md): V1 and V2 lamps, V2 calibration and centring.

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
| `orion-service/` | Pi voice/agent host and Studio remote client |
| `runtime/` | Rust `oriond` daemon, hardware and MuJoCo backends, lifecycle, scenes, lighting, and playback |
| `coordinator/` | Reusable Rust voice orchestration, Pi transport, buffering, and playback lifecycle |
| `speech/` | CPU Qwen and Piper workers plus optional Apple Silicon development adapters |
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
cargo test --manifest-path runtime/Cargo.toml --all-targets
PYTHONPATH=motion .venv/bin/python -m pytest -q motion/test
python3 -m unittest discover -s orion_studio/tests -v

cd orion_studio
pnpm test
pnpm build
```

Some runtime integration tests expect the repository Python environment at
`.venv/bin/python`. Model-independent voice-worker tests use the worker's own
environment; see its [validation instructions](speech/README.md#validation).
Most Orion environments use Python 3.12; Pi speech uses Python 3.11 for its
CPU dependencies. Package metadata and setup scripts select the versions.
The root `.venv` supports workstation simulation. Keep separate environments for Pi
capture (`voice/.venv`), speech inference (`speech/.venv`), and
servo setup (`hardware/servo_setup/.venv`). Use `uv sync --locked` for
packages with a lockfile. The Pi gateway uses system Python. The release preparer creates separate
environments for capture and inference.

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
[servo setup guide](hardware/servo_setup/README.md). Software torque
disable is not an emergency stop. Keep an accessible physical power or torque
interruption available during hardware work.
