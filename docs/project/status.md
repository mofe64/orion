# Orion implementation status

Status meanings:

- **Implemented:** present in the repository and covered by automated or
  recorded physical validation.
- **Partial:** a useful slice exists, but an important boundary or deployment
  requirement remains.
- **Planned:** no supported end-to-end implementation exists yet.

## Implemented

| Area | Evidence and scope |
| --- | --- |
| Robot description | Neutral URDF and shared STL meshes under `description/` |
| Motion foundation | V2 named poses and motions, commissioned calibration limits, one continuous Rust spline compiler, and the documented [motion architecture](../explanation/motion-and-animation-architecture.md) and [control reference](../reference/trajectory-and-joint-control.md) |
| Native runtime | Rust `oriond` daemon with lifecycle, run IDs, status, settling, cancellation, and hardware/MuJoCo backends |
| Physical servo path | Five STS3215 servos through the commissioned calibration and `rustypot` transport |
| Lighting | Pi 5 BCM12 RGBW output, logical RGBW conversion, fades, and all-off handling |
| Local audio | Named WAV cues and `oriond`-owned ReSpeaker playback |
| Character coordinator | Explicit disabled/idle/listening/thinking/speaking states, priority, anchor-relative idles, head-led utterance-length speech performance, and background lighting |
| Multimodal scenes | V2 parallel motion, RGBW effect, marker, audio, and exact finish-policy coordination under one clock |
| Pi services and deployment | Source-backed `oriond` and Studio gateway systemd units plus bounded deployment smoke test |
| Orion Studio authoring | Responsive dark creative workspace, URDF preview, v2 pose/motion/scene editors, Rust-compiled preview, calibrated controls, and revisioned publishing |
| Studio-to-Pi control | Authenticated semantic HTTP gateway over the private `oriond` Unix socket |
| Studio Voice response | Workstation capture, resampling, Rustpotter activation, Qwen3-ASR confirmation/transcription, provider-isolated text agent, Chatterbox generation, authenticated Pi upload, and ReSpeaker completion reporting |
| Pi-local fallback voice | Sherpa wake detection, Silero endpointing, Moonshine transcription, Piper speech generation, and `oriond` playback integration |

## Partial

| Area | What exists | What remains |
| --- | --- | --- |
| Studio platform support | Tauri targets macOS, Windows, and Linux | Qwen/Chatterbox MLX inference is Apple-Silicon-only; Windows and Linux packages are not fully commissioned |
| Agent abstraction | A small `AgentProvider` boundary and Codex provider exist | OpenAI Platform and local-LLM providers are not implemented |
| Conversational privacy | Raw audio, ASR, and TTS stay local | The Codex provider sends confirmed text to a cloud service; provider disclosure and product policy must remain explicit |
| Voice packaging | Source development starts a persistent local worker | Signed installers do not package Python, the native extension, or model weights |
| Pi audio front end | Commissioned 16 kHz mono ReSpeaker capture | Stereo capture, beamforming, echo cancellation, and noise suppression remain planned |
| Studio network security | Bearer-token authentication and exact development origins | Production pairing, encrypted transport, certificate identity, and token lifecycle hardening remain |

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
