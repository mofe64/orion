# Orion

Orion is an expressive robotic lamp. A Raspberry Pi runs the safety-critical
hardware runtime, while Orion Studio provides scene authoring, robot control,
and the primary workstation voice experience.

## Start here

Choose the path that matches what you want to do:

- [Understand the system](docs/explanation/system-architecture.md) — component
  boundaries, data flow, and safety ownership.
- [Build and run the simulator](docs/tutorials/first-runtime-run.md) — the
  shortest hardware-free path to a working Orion runtime.
- [Run Orion Studio](docs/tutorials/first-studio-run.md) — install the desktop
  dependencies and open the application.
- [Set up Studio Voice](docs/tutorials/first-studio-voice-run.md) — install the
  local worker, pre-download its models, and test the complete voice path.
- [Deploy to the Raspberry Pi](runtime/README.md#deploy-an-update-to-the-raspberry-pi)
  — update the source-backed services and run the bounded hardware smoke test.
- [Browse all documentation](docs/README.md) — tutorials, how-to guides,
  explanations, reference material, project status, and learning notes.

## System at a glance

```text
Workstation                                      Raspberry Pi

Orion Studio                                     authenticated HTTP gateway
├── scene and motion editor     semantic API     ├── named assets and actions
├── local microphone          ────────────────▶  └── private Unix socket
├── wake + ASR + agent + TTS                            │
└── local speaker                                      ▼
                                                    oriond
                                                    ├── lifecycle and safety
                                                    ├── motion and scenes
                                                    ├── STS3215 servos
                                                    ├── RGBW lighting
                                                    └── ReSpeaker playback
```

`oriond` is the sole authority for Raspberry Pi hardware. Studio and future
agents submit semantic requests such as named poses, motions, scenes, and
speech; they never write servo registers or GPIO directly.

Studio Voice is the primary interactive voice path. It captures from the
workstation microphone and runs Rustpotter, Qwen3-ASR, the configured agent,
and Chatterbox locally around an authenticated loopback connection. The older
Pi-local Piper/Sherpa/Moonshine stack remains available as a diagnostic and
offline fallback. See the [voice architecture](docs/explanation/voice-architecture.md)
for the distinction.

## Repository map

| Path | Responsibility |
| --- | --- |
| `runtime/` | Rust `oriond` daemon, hardware and MuJoCo backends, lifecycle, scenes, lighting, and playback |
| `orion_studio/` | Tauri/React desktop application, Pi gateway, and primary voice worker |
| `motion/` | Shared poses, motions, limits, trajectory logic, and tests |
| `scenes/` | Versioned multimodal scene documents |
| `description/` | Neutral URDF and shared mesh assets |
| `simulation/mujoco/` | MuJoCo model, playback tools, and simulator checks |
| `hardware/` | Commissioning and operating instructions for servos, audio, and lighting |
| `voice/` | Raspberry Pi-local fallback wake, ASR, and Piper TTS processes |
| `audio/` | Named local audio cues |
| `docs/` | Cross-system documentation, project status, decisions, and learning material |

Training recordings and wake-word evaluation tools are not runtime
dependencies. They live in the separate sibling `voice-model-lab` workspace.

## Common validation commands

Run these from the repository root unless a linked guide says otherwise:

```bash
cargo test --manifest-path runtime/Cargo.toml --all-targets
PYTHONPATH=motion .venv/bin/python -m pytest -q motion/test
python3 -m unittest discover -s orion_studio/tests -v

cd orion_studio
pnpm test
pnpm build
```

Some runtime integration tests expect the repository Python environment at
`.venv/bin/python`. Model-independent voice-worker tests use the worker's own
environment; see its [validation instructions](orion_studio/voice_worker/README.md#validation).

## Current scope

The runtime, simulator, Pi hardware path, scene system, Studio authoring and
gateway, and Studio speech-response pipeline are implemented. Deterministic
agent-to-robot capability routing, production network pairing, packaged voice
models, and non-Apple-Silicon Studio inference remain incomplete. The
remaining work is listed in [project status](docs/project/status.md).

## Safety

Before powering or moving physical hardware, follow the
[servo commissioning guide](hardware/servo_setup/README.md). Software torque
disable is not an emergency stop. Keep an accessible physical power or torque
interruption available during hardware work.
