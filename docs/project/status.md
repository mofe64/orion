# Orion implementation status

Status meanings:

- **Implemented:** present in the repository and covered by automated or
  recorded physical validation.
- **Partial:** a useful slice exists, but an important boundary or deployment
  requirement remains.
- **Planned:** no supported end-to-end implementation exists.

Orion uses the Unified Robot Description Format (URDF) for its neutral model
and red-green-blue-white (RGBW) channels for expressive light. Studio Voice
uses automatic speech recognition (ASR), text-to-speech (TTS), and Apple's MLX
machine-learning framework.

## Implemented

| Area | Evidence and scope |
| --- | --- |
| Robot description | Neutral URDF model and shared STL meshes under `description/` |
| Motion foundation | V2 named poses and motions, commissioned calibration limits, one continuous Rust spline compiler, and the documented [motion architecture](../explanation/motion-and-animation-architecture.md) and [control reference](../reference/trajectory-and-joint-control.md) |
| Native runtime | Rust `oriond` daemon with lifecycle, run IDs, status, settling, cancellation, and hardware/MuJoCo backends |
| Physical servo path | Five STS3215 servos through the commissioned calibration and `rustypot` transport |
| Lighting | Pi 5 BCM12 RGBW output, logical conversion, fades, and all-off handling |
| Local audio | Named WAV cues and `oriond`-owned ReSpeaker playback |
| Character coordinator | Explicit disabled/idle/listening/thinking/speaking states, priority, anchor-relative idles, head-led utterance-length speech performance, and background lighting |
| Multimodal scenes | V2 parallel motion, RGBW effect, marker, audio, and exact finish-policy coordination under one clock |
| Pi services and deployment | Source-backed runtime/gateway/listener services, incremental locked Rustpotter installation, plus bounded deployment smoke test |
| Orion Studio home and authoring | Home/Create navigation; character, rest, lamp power, warm-white/custom-color brightness, a rotatable home model, and voice controls; per-asset drafts; v2 pose/motion/scene editors; Rust-compiled preview; run-specific cancellation; revisioned publishing |
| Studio-to-Pi control | Authenticated semantic HTTP gateway over the private `oriond` Unix socket; OS-stored desktop pairing and automatic reconnect |
| Studio Voice response | Pi Rustpotter capture transport, Studio Qwen confirmation/agent/Chatterbox, authenticated Pi playback, and completion reporting |

## Partial

| Area | What exists | What remains |
| --- | --- | --- |
| Studio platform support | Tauri targets macOS, Windows, and Linux | MLX voice inference is Apple-Silicon-only; Windows and Linux packages are uncommissioned |
| Agent abstraction | A small `AgentProvider` boundary and Codex provider exist | OpenAI Platform and local-LLM providers are not implemented |
| Conversational privacy | Raw audio, ASR, and TTS stay local | Codex receives confirmed text; provider disclosure must remain explicit |
| Voice packaging | Source development starts a persistent local worker | Signed installers do not package Python, the native extension, or model weights |
| Streaming speech | Ordered gateway chunk uploads, one Pi player, incremental character spline and per-stage diagnostics | Physical buffer, playback alignment and animation acceptance |
| Pi audio front end | Stereo capture and coarse direction software; earlier mono hardware commissioning | Stereo orientation, Pi Rustpotter performance and physical attention acceptance; echo cancellation and noise suppression remain |
| Studio network security | Bearer tokens and development origins; plain LAN WebSocket microphone transport | Device-approved initial pairing, encrypted voice/gateway transport and token lifecycle hardening remain |

## Planned

| Area | Required outcome |
| --- | --- |
| Deterministic agent capability routing | Convert agent intent into an allow-listed, validated Orion capability request before any physical action |
| Perception and world model | Provide explicit, confidence-bearing observations for attention and behaviour systems |
| Task-space control | Add validated target-pointing behaviour without bypassing joint limits or runtime ownership |
| Behaviour orchestration | Coordinate attention, motion, lighting, sound, and interruption through explicit state rather than direct device calls |
| Context-aware expression | Apply the ELEGNT model to modulate timing and expression without compromising task clarity or safety |
| Product hardening | Complete packaging, production pairing, recovery, evaluation, privacy policy, and release evidence |
| Custom Orion hardware | Move beyond the LeLamp-compatible prototype only after behaviour requirements justify mechanical changes |
