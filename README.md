# Orion

Orion is an expressive robotic lamp. A Raspberry Pi runs motion, camera processing, speech recognition, speech synthesis and the Codex coordinator. Studio provides robot controls, scene editing and settings.

Speech processing runs on the Pi. Codex replies and web search require internet access. Closing Studio does not stop Orion.

## First actions

1. Install Orion with the [Pi quickstart](docs/quickstart.md).
2. Set servo IDs and joint zero with [hardware setup](docs/hardware-versions.md#calibrate-v2-first).
3. Connect [Studio](orion_studio/README.md#connect-studio-to-the-pi).

## System flow

```text
microphone → wake detection → command recording → speech recognition
                                                       ↓
speaker + movement ← oriond ← gateway ← speech synthesis ← Codex
```

The hardware runtime, `oriond`, owns servo writes, light output and audio playback. Other processes send requests through checked interfaces. See [system architecture](docs/system-architecture.md).

## Repository map

| Directory | Function |
| --- | --- |
| `agent/` | Codex sessions, tools and memory |
| `history/` | Conversation files and behavior journal |
| `orion-service/` | Pi service host and Studio client |
| `coordinator/` | Voice session control and Pi transport |
| `runtime/` | Hardware control, simulation and playback |
| `speech/` | Local speech recognition and synthesis |
| `voice/` | Microphone capture and wake detection |
| `vision/` | Camera processing and target selection |
| `orion_studio/` | Desktop application and Pi gateway |
| `motion/` | Poses, motions and trajectory consumers |
| `scenes/` | Combined motion, light and audio assets |
| `description/` | Robot description and meshes |
| `simulation/mujoco/` | MuJoCo models and tools |
| `hardware/` | Profiles and hardware setup |
| `audio/` | Local sound assets |
| `docs/` | Architecture, references and learning notes |

## Development

Each Python component uses uv-managed Python 3.12. Installed releases use `/opt/orion/python`. The system Python does not run Orion components.

1. Run the Rust runtime tests from the repository root.

   ```bash
   cargo test --manifest-path runtime/Cargo.toml --all-targets --locked
   ```

2. Follow the component READMEs for their dependencies and tests.

The [documentation index](docs/README.md) links to build instructions and configuration references.

## Operating constraints

The gateway uses a bearer token over HTTP on a trusted local network. A bearer token grants access to its holder. See [connection settings](docs/configuration.md#studio-gateway).

Orion accepts a follow-up after reply playback ends. V2 also accepts verified wake phrases during playback. See [voice interruption](docs/voice-architecture.md#wake-phrase-interruption).

WARNING: Powered joints can pinch fingers. Keep a physical power disconnect accessible during hardware work.

1. Follow [hardware setup](docs/hardware-versions.md#calibrate-v2-first) before operating powered joints.

## System readings

Studio provides a System page for Pi resource use and service state. See the [Studio guide](orion_studio/README.md#system-readings).
