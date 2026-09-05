# Orion roadmap

## Implemented foundation

Orion has a neutral robot description, shared motion assets, a Rust runtime,
MuJoCo parity, commissioned Pi hardware, multimodal scenes, a desktop authoring
tool, and Studio-processed voice with Pi capture and playback.



## Todo

1. **Make development reproducible.** Make a fresh clone, simulator run,
   Studio launch, Voice setup, and Pi deployment reliable.
2. **Define the agent capability boundary.** Introduce a deterministic,
   allow-listed interface between interpreted intent and `oriond` semantic
   operations, with confirmation and denial behaviour.
3. **Harden Studio deployment.** Package its worker dependencies and model
   installation, commission supported platforms, and replace the
   development LAN bearer-token arrangement.
4. **Measure voice quality.** Evaluate wake false accepts/rejects, endpointing,
   speech-recognition accuracy, response latency, playback interruption, and echo behaviour
   using retained test evidence outside the runtime repository.
5. **Add perception and attention state.** Establish explicit observations and
   confidence before introducing autonomous attention behaviours.
6. **Add task-space and behaviour orchestration.** Keep all generated movement
   inside the existing validation, lifecycle, and hardware ownership boundary.
7. **Apply context-aware ELEGNT expression.** Treat expression as bounded
   modulation of approved behaviour, not an independent source of physical
   commands.



Optional experiments—advanced sensing, richer generative behaviour, or
additional model providers—must not displace safety, reliability, or the
ordered product outcomes above.
