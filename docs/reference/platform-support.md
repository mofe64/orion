# Orion platform support

Application support and local model-inference support differ. Building the
desktop shell on an operating system does not make every voice adapter
available there.

| Component | macOS Apple Silicon | Windows | Linux workstation | Raspberry Pi 5 |
| --- | --- | --- | --- | --- |
| Orion Studio UI and editor | Supported | Build target; not fully commissioned | Build target; not fully commissioned | Not a target |
| Studio-to-Pi gateway client | Supported | Expected; not fully commissioned | Expected; not fully commissioned | Gateway server only |
| Rustpotter reference detection | Supported | Native crate is portable; integration not commissioned | Native crate is portable; integration not commissioned | Not used by fallback stack |
| Qwen3-ASR MLX adapter | Supported | Unsupported | Unsupported | Unsupported |
| Chatterbox MLX adapter | Supported | Unsupported | Unsupported | Unsupported |
| Pi-local Sherpa/Moonshine/Piper stack | Not the primary target | Not a target | Not the primary target | Supported fallback |
| `oriond` MuJoCo backend | Supported development path | Not commissioned | Expected development path | Not required |
| `oriond` physical backend | Not a target | Not a target | Not a target | Supported |

“Build target” means the Tauri configuration can produce an application for
that operating system. “Supported” means automated tests cover the path.
“Commissioned” means it has also been exercised on the named physical
platform.

The implemented Qwen3-ASR and Chatterbox adapters depend on Apple's MLX
machine-learning framework and therefore need an Apple Silicon Mac. Supporting
Studio Voice on Windows or Linux requires a non-MLX inference adapter; the
worker protocol and `AgentProvider` boundary do not otherwise require macOS.

Version-specific dependencies live in their manifests:

- Studio JavaScript dependencies: [`orion_studio/package.json`](../../orion_studio/package.json)
- Studio Voice Python dependencies: [`voice_worker/pyproject.toml`](../../orion_studio/voice_worker/pyproject.toml)
- Runtime Rust dependencies: [`runtime/Cargo.toml`](../../runtime/Cargo.toml)
