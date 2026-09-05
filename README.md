# Orion

Orion is an expressive robotic lamp. It uses a Raspberry Pi as the onboard computer for running the safety-critical hardware runtime, while Orion Studio provides scene authoring, robot control,
and acts as the external voice processing and agent runtime.

## Start here

- [Understand the system](docs/explanation/system-architecture.md) — component
  boundaries, data flow, and safety ownership.
- [Understand motion and animation](docs/explanation/motion-and-animation-architecture.md)
  — character intent, continuous trajectories, runtime execution, and joint
  control.
- [Understand character animation](docs/explanation/character-animation.md) —
  the 12 principles, autonomous idle, and speech-driven performance.
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

Orion Studio acts as the external processing station and enables its voice functionality, which combines automatic speech recognition (ASR), AI agent, and text-to-speech (TTS).
Orion also posseses functional and expressive lighting via a led (adafruit neo pixel shield) with red, green, blue, and dedicated
white channels.

```text
External Computer                                Onboard Computer (Raspberry Pi)

Orion Studio                                     authenticated HTTP gateway
├── scene and motion editor     semantic API     ├── named assets and actions
├── Pi audio processing       ────────────────▶  └── private Unix socket
├── Qwen ASR + agent + TTS                            │
└── validated WAV upload                               ▼
                                                    oriond
                                                    ├── lifecycle and safety
                                                    ├── character + scenes
                                                    ├── continuous motion
                                                    ├── STS3215 servos
                                                    ├── RGBW lighting
                                                    └── ReSpeaker playback
```

`oriond` is the active runtime for Onboard computer. Studio submits
semantic requests such as named poses, motions, scenes, and speech, but does not control
hardware directly. AI agent integrations use the same semantic boundary.

Studio Voice processes audio captured exclusively by the onboard Pi. The Pi
runs Rustpotter and forwards bounded utterances over the local network; Studio confirms the
wake with Qwen3-ASR, invokes the configured agent, and synthesizes replies with
Chatterbox. Studio provides the compute for speech recognition, the agent, and
expressive synthesis; the Pi plays replies and owns character animation. See the
[voice architecture](docs/explanation/voice-architecture.md) and
[Pi setup](voice/README.md).

## Repository map

| Path | Responsibility |
| --- | --- |
| `runtime/` | Rust `oriond` daemon, hardware and MuJoCo backends, lifecycle, scenes, lighting, and playback |
| `orion_studio/` | Tauri/React desktop application, Pi gateway, and primary voice worker |
| `motion/` | Pose and motion assets plus Python consumers of Rust-compiled trajectories |
| `scenes/` | Versioned multimodal scene documents |
| `description/` | Neutral Unified Robot Description Format (URDF) model and shared mesh assets |
| `simulation/mujoco/` | MuJoCo model, playback tools, and simulator checks |
| `hardware/` | Commissioning and operating instructions for servos, audio, and lighting |
| `voice/` | Pi microphone capture, Rustpotter wake detection, and Studio transport |
| `audio/` | Named local audio cues |
| `docs/` | Cross-system documentation, project status, decisions, and learning material |


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
Orion-managed environments use Python 3.12, selected by `.python-version` and
package metadata. The simulator environment (`.venv`) is for workstation development and is not
installed or required on the Pi. Keep separate environments for Pi
capture (`voice/.venv`), Studio inference (`orion_studio/voice_worker/.venv`), and
servo commissioning (`hardware/servo_setup/.venv`). Use `uv sync --locked` for
packages with a lockfile. The Pi gateway and the `uv` bootstrap use system Python;
they do not require changing the operating system interpreter.

## Implementation status

Orion implements the runtime, simulator, Pi hardware path, scene system, Studio
authoring and gateway, and Studio speech-response pipeline. Deterministic
agent-to-robot capability routing is planned. Production network pairing,
packaged voice models, and non-Apple-Silicon Studio inference remain partial.
See [project status](docs/project/status.md) for each capability boundary.

## Safety

Before powering or moving physical hardware, follow the
[servo commissioning guide](hardware/servo_setup/README.md). Software torque
disable is not an emergency stop. Keep an accessible physical power or torque
interruption available during hardware work.
