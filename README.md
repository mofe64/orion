# Orion

Orion is an expressive robotic lamp. Its Raspberry Pi runs the hardware runtime,
local speech recognition and synthesis, and the Codex agent coordinator. Studio
provides scene authoring, robot controls, and voice settings. Speech works with
Studio closed; Codex inference and web search still require the internet.

## Start here

- [Quickstart](docs/quickstart.md) — open Studio and install or update the Pi services.
- [Understand the system](docs/system-architecture.md) — component
  boundaries, data flow, and safety ownership.
- [Understand motion and animation](docs/motion-and-animation-architecture.md)
  — character intent, continuous trajectories, runtime execution, and joint
  control.
- [Understand character animation](docs/character-animation.md) —
  the 12 principles, autonomous idle, and speech-driven performance.
- [Build the runtime](runtime/README.md#build-and-test) and
  [run it in MuJoCo](runtime/README.md#mujoco-first-daemon).
- [Run Orion Studio](orion_studio/README.md#development) — install the desktop
  dependencies and open the application.
- [Set up Pi voice and agent](docs/quickstart.md#pi-local-voice-and-agent) — install the
  inference worker and models, then [connect Pi capture](voice/README.md#setup).
- [Deploy to the Raspberry Pi](runtime/README.md#deploy-an-update-to-the-raspberry-pi)
  — update the full stack while preserving Pi settings and a recent rollback.
- [Browse all documentation](docs/README.md) — architecture, reference material,
  component setup, and learning notes.

## System at a glance

The Pi voice pipeline uses Rustpotter → Silero → Qwen3-ASR → Codex → Pocket TTS.
The same hardware runtime owns motion, speech animation, and RGBW lighting.

```text
Studio (optional)  ── authenticated gateway ── Pi settings and robot controls
                                               │
Pi microphone → Rustpotter + Silero → Qwen → Codex App Server → Pocket TTS
                                               │                   │
                                       online model/search         ▼
                                                   oriond streaming playback
                                                   + motion and RGBW lighting
```

`oriond` is the active runtime for Onboard computer. Studio submits
semantic requests such as named poses, motions, scenes, and speech, but does not control
hardware directly. AI agent integrations use the same semantic boundary.

The onboard coordinator confirms wake candidates with Qwen, calls the Rust
agent, and synthesizes Pocket replies. `oriond` owns character animation and
acknowledges actual playback. Studio observes the Pi and selects saved voice
presets without taking over processing. See the [voice architecture](docs/voice-architecture.md)
and [Pi setup](voice/README.md).

## Repository map

| Path | Responsibility |
| --- | --- |
| `agent/` | Rust conversation runtime, memory/tools, and Codex integration |
| `orion-service/` | Pi voice/agent host and Studio remote client |
| `runtime/` | Rust `oriond` daemon, hardware and MuJoCo backends, lifecycle, scenes, lighting, and playback |
| `coordinator/` | Reusable Rust voice orchestration, Pi transport, buffering, and playback lifecycle |
| `speech/` | CPU Qwen/Pocket workers and legacy Apple Silicon adapters |
| `orion_studio/` | Tauri/React desktop application and Pi gateway |
| `motion/` | Pose and motion assets plus Python consumers of Rust-compiled trajectories |
| `scenes/` | Versioned multimodal scene documents |
| `description/` | Neutral Unified Robot Description Format (URDF) model and shared mesh assets |
| `simulation/mujoco/` | MuJoCo model, playback tools, and simulator checks |
| `hardware/` | Commissioning and operating instructions for servos, audio, and lighting |
| `voice/` | Pi microphone capture, Rustpotter, Silero endpointing, and coordinator transport |
| `audio/` | Named local audio cues |
| `docs/` | Cross-system architecture, configuration, animation references, and learning notes |


## Common validation commands

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
CPU dependencies. Versions are selected by package metadata and setup scripts. The simulator environment (`.venv`) is for workstation development and is not
installed or required on the Pi. Keep separate environments for Pi
capture (`voice/.venv`), speech inference (`speech/.venv`), and
servo commissioning (`hardware/servo_setup/.venv`). Use `uv sync --locked` for
packages with a lockfile. The Pi gateway and the `uv` bootstrap use system Python;
they do not require changing the operating system interpreter.

## Implementation status

Orion implements the runtime, simulator, Pi hardware path, scene system, Studio
authoring and gateway, and the Pi speech-response pipeline. Allowlisted agent
lighting commands are implemented; broader agent motion capabilities are planned. Production network pairing,
portable model packaging and broader offline agent support remain partial.
Component READMEs describe setup, validation, and remaining platform constraints.

## Safety

Before powering or moving physical hardware, follow the
[servo commissioning guide](hardware/servo_setup/README.md). Software torque
disable is not an emergency stop. Keep an accessible physical power or torque
interruption available during hardware work.
