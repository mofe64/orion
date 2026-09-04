# Handover Reports Index

> This index is the retrieval layer for Orion handovers. Reports are ordered newest first; use the topic and status sections to find the authoritative context for the work in front of you.

**Last Updated:** 2026-09-04
**Total Reports:** 5

---

## Quick Search

### By Topic

- [Testing](#testing)
- [Bug Fixes](#bug-fixes)
- [Features](#features)
- [Refactoring](#refactoring)
- [Documentation](#documentation)
- [Hardware](#hardware)
- [Native Runtime](#native-runtime)
- [Lighting, Audio, and Scenes](#lighting-audio-and-scenes)
- [Voice and Orion Studio](#voice-and-orion-studio)
- [Simulation](#simulation)
- [ROS 2 History](#ros-2-history)

### By Status

- [Complete](#complete)
- [In Progress](#in-progress)
- [Blocked](#blocked)

---

## All Reports (Reverse Chronological)

### 2026-09-04 - Pi Rustpotter, Studio processing and character attention ⏳

**File:** [handover-2026-09-04.md](./handover-2026-09-04.md)

**Primary Task:** Implement Pi-owned capture/wake, Studio Qwen confirmation and processing, default character startup and ELEGNT attention.

**Tags:** `voice`, `rustpotter`, `raspberry-pi`, `attention`, `animation`, `elegnt`, `testing`

**Key Deliverables:** Pi token-authenticated LAN listener, complete voice deployment/legacy retirement, persistent OS pairing/reconnect, Studio home audit, processing-only Studio, character startup and authored attention previews.

**Bugs Fixed:** Startup failure classification, follow-up buffering, delayed attention return, upload cancellation and stale UI readiness.

**Files Changed:** Voice packages, Studio integration, runtime, motion assets, service/deployment scripts and documentation; pre-existing user edits retained.

**Next Steps:** Deploy through the updated single-command workflow, validate Pi pairing/reconnect and stereo/wake/physical attention; review the nine prioritized home UI proposals.

### 2026-08-31 - Movement lifecycle, multimodal runtime, and local voice foundation complete ⏳

**File:** [handover-2026-08-31-multimodal-voice.md](./handover-2026-08-31-multimodal-voice.md)

**Primary Task:** Add trustworthy movement acknowledgement, commission physical RGBW lighting and ReSpeaker V2 audio, implement local multimodal scenes and Piper speech, prove Pi-local wake/transcription, and define the Orion Studio continuation boundary.

**Tags:** `orion`, `movement-lifecycle`, `settling`, `neopixel`, `respeaker-v2`, `audio`, `scenes`, `piper`, `wake-word`, `speech-to-text`, `orion-studio`, `raspberry-pi`

**Key Deliverables:**

- Ephemeral movement IDs, measured settling, terminal results, `--wait`, meaningful exit codes, and corrected busy-motion errors
- Commissioned 40-pixel RGBW lighting, ReSpeaker V2 playback/capture, persistent hardware prerequisites, local cues, and four versioned scenes
- Piper Ryan Medium speech lifecycle plus a physically proven Sherpa/Silero/Moonshine wake-and-transcription pipeline

**Bugs Fixed:** 10

**Files Changed:** Rust runtime lifecycle/device/scene/speech modules, lighting and audio hardware setup, portable scene/cue assets, Python voice runtime/tests, documentation, plus this report/index update

**Next Steps:** Define the network-facing semantic capability protocol, scaffold Orion Studio, prove Mac push-to-talk plus larger-model transcription, then add wake detection and agent routing while retaining the Pi voice path as fallback/diagnostics.

### 2026-08-29 - Rust runtime port and physical parity complete ✅

**File:** [handover-2026-08-29-rust-runtime.md](./handover-2026-08-29-rust-runtime.md)

**Primary Task:** Port the active native runtime to Rust, validate MuJoCo and physical feature parity, remove the superseded C++ implementation, and make `runtime/` the sole runtime package.

**Tags:** `orion`, `rust`, `rustypot`, `native-runtime`, `sts3215`, `mujoco`, `hardware`, `feature-parity`, `runtime-removal`, `documentation`, `raspberry-pi`

**Key Deliverables:**

- Pure-Rust physical STS3215 transport and complete Rust `oriond` lifecycle/motion implementation
- Rust-owned MuJoCo backend, 182 passing workstation tests, and successful physical pose/motion/stop/rest/disable trial
- Complete C++ runtime removal, canonical `runtime/` package name, and current Rust-only operator documentation

**Bugs Fixed:** 6

**Files Changed:** Rust runtime crate, complete C++ runtime/vendor removal, current documentation, plus this report/index update

**Next Steps:** Confirm the next slice with the user; recommended order is LED and ReSpeaker device boundaries, local multimodal scene schema/player, then Orion Studio.

### 2026-08-29 - Orion native runtime, recalibration, and ROS 2 removal ✅

**File:** [handover-2026-08-29.md](./handover-2026-08-29.md)

**Primary Task:** Recalibrate the assembled robot, align shared poses and MuJoCo with the physical reference, implement and physically validate the native C++ runtime, and remove the active ROS 2 dependency.

**Tags:** `orion`, `native-runtime`, `c++`, `sts3215`, `calibration`, `rest-pose`, `motions`, `mujoco`, `ros2-removal`, `hardware`, `testing`

**Key Deliverables:**

- Accepted five-joint calibration and hardware-derived torque-free rest pose
- Native `oriond` lifecycle, status, named-pose, multi-keyframe motion, stop, and disable paths
- Shared ROS-independent `motion/` and `description/` assets consumed by hardware and MuJoCo
- Physical pose/motion/rest smoke test plus 174 passing portable/native tests

**Bugs Fixed:** 9

**Files Changed:** 180 files across the work since the previous handover baseline, plus this report/index update

**Next Steps:** Add status-aware sequencing, correct the busy-motion error, harden tracking/watchdog/obstacle handling, and physically tune the slowed right expressive motion.

### 2026-08-28 - Orion hardware bring-up and physical-control handoff ⏳

**File:** [handover-2026-08-28.md](./handover-2026-08-28.md)

**Primary Task:** Assemble and commission the five-servo Orion lamp, capture initial calibration/rest data, and prepare the first physical control backend.

**Tags:** `orion`, `sts3215`, `servo-setup`, `calibration`, `rest-pose`, `pose-execution`, `ros2-control`, `mujoco`, `hardware`, `safety`

**Key Deliverables:**

- Five-servo provisioning, verification, first-motion, calibration, rest-capture, and guarded pose-runner workflows
- Initial compiled/unit-tested `ros2_control` backend and the evidence that motivated the native runtime

**Bugs Fixed:** 6

**Files Changed:** 41 across the commissioning series, plus its report/index update

**Next Steps at the time:** Commission ROS control and characterize torque. These directions were superseded by the 2026-08-29 native-runtime handover; retain this report as bring-up history.

---

## Topic Index

### Testing

- [2026-09-04](./handover-2026-09-04.md) — Pi voice, startup and attention; physical acceptance pending.

- [2026-08-31 multimodal/voice handover](./handover-2026-08-31-multimodal-voice.md) - Records the 239-test matrix, deterministic lifecycle/scene/voice coverage, physical light/audio/scene evidence, and successful but imperfect Pi transcription samples.
- [2026-08-29 Rust handover](./handover-2026-08-29-rust-runtime.md) - Records 182 passing workstation tests, the optional Pi MuJoCo/Python distinction, and successful Rust hardware pose, motion, stop, rest, and disable evidence.
- [2026-08-29](./handover-2026-08-29.md) - Records 174 passing tests, a ROS-free native build, a valid MuJoCo load, and the successful post-cleanup physical smoke sequence.
- [2026-08-28](./handover-2026-08-28.md) - Records 46 commissioning tests, strict nudge results, and the original elbow/pose tracking failures.

### Bug Fixes

- [2026-09-04](./handover-2026-09-04.md) — Pi voice, startup and attention; physical acceptance pending.

- [2026-08-31 multimodal/voice handover](./handover-2026-08-31-multimodal-voice.md) - Fixes busy-motion reporting, persistent NeoPixel setup, wrong ReSpeaker generation, unstable mixer state, Chatterbox performance, Sherpa dependencies, ALSA microphone ownership, and overdriven capture gain.
- [2026-08-29 Rust handover](./handover-2026-08-29-rust-runtime.md) - Resolves the parallel runtime tree, old MuJoCo bridge path, stale C++ ownership documentation, shutdown ordering, and optional Python-environment diagnosis.
- [2026-08-29](./handover-2026-08-29.md) - Fixes calibration display/range handling, supported endpoints, margin starts, simulation reference alignment, stale pose families, command errors, Pi Git divergence, and backend paths.
- [2026-08-28](./handover-2026-08-28.md) - Fixes early yaw calibration rejection, rest shutdown behaviour, shared rest ranges, Ctrl+C parking, and temperature diagnostics.

### Features

- [2026-09-04](./handover-2026-09-04.md) — Pi voice, startup and attention; physical acceptance pending.

- [2026-08-31 multimodal/voice handover](./handover-2026-08-31-multimodal-voice.md) - Adds movement acknowledgement, RGBW control, WAV playback, local multimodal scenes, Piper speech, wake detection, VAD, and command transcription.
- [2026-08-29 Rust handover](./handover-2026-08-29-rust-runtime.md) - Rust daemon, `rustypot` hardware transport, shared hardware/MuJoCo boundary, source-tree Pi workflow, and verified physical feature parity.
- [2026-08-29](./handover-2026-08-29.md) - Native daemon, physical pose/motion playback, constrained MuJoCo pose editor, supported-rest capture, and rebuilt expressive motions.
- [2026-08-28](./handover-2026-08-28.md) - Servo provisioning, verification, guarded motion, initial calibration, rest capture, and physical pose commissioning.

### Refactoring

- [2026-08-29 Rust handover](./handover-2026-08-29-rust-runtime.md) - Removes the C++ runtime/vendor SDK and renames the sole Rust crate from `runtime_rust/` to `runtime/`.
- [2026-08-29](./handover-2026-08-29.md) - Moves shared assets to `motion/` and `description/`, removes `ros2_ws/`, archives ROS documents, and deduplicates meshes.

### Documentation

- [2026-09-04](./handover-2026-09-04.md) — Pi voice, startup and attention; physical acceptance pending.

- [2026-08-31 multimodal/voice handover](./handover-2026-08-31-multimodal-voice.md) - Current source of truth for movement completion, lighting/audio hardware, scenes, local voice, physical evidence, known limitations, and the Studio continuation.
- [2026-08-29 Rust handover](./handover-2026-08-29-rust-runtime.md) - Current source of truth for the Rust-only architecture, Pi source workflow, optional MuJoCo Python dependency, shutdown, and continuation order.
- [2026-08-29](./handover-2026-08-29.md) - Records the accepted hardware contract, native architecture, current commands, migration evidence, risks, and next runtime milestone.
- [2026-08-28](./handover-2026-08-28.md) - Consolidates original assembly, wiring, calibration, safety, and planned control-backend context.

### Hardware

- [2026-08-31 multimodal/voice handover](./handover-2026-08-31-multimodal-voice.md) - Records commissioned RGBW matrix mapping, persistent RP1 PWM setup, ReSpeaker V2 identity, JST playback, microphone capture, and physical multimodal scenes.
- [2026-08-29 Rust handover](./handover-2026-08-29-rust-runtime.md) - Records Rust Pi telemetry parity and the physical configure, enable, pose, motion, stop, rest, and disable sequence.
- [2026-08-29](./handover-2026-08-29.md) - Authoritative calibration, supported endpoints, 120 g head correction, captured rest, servo profile, and real motion evidence.
- [2026-08-28](./handover-2026-08-28.md) - Initial five-servo assembly, wiring, bus detection, and bring-up constraints.

### Native Runtime

- [2026-08-31 multimodal/voice handover](./handover-2026-08-31-multimodal-voice.md) - Extends the Rust runtime source of truth with movement run IDs, measured settling, lighting/audio ownership, scenes, and generated-speech lifecycles.
- [2026-08-29 Rust handover](./handover-2026-08-29-rust-runtime.md) - Current source of truth for the sole Rust `oriond`, `rustypot` STS3215 ownership, physical parity, source-tree operation, and known gaps.
- [2026-08-29](./handover-2026-08-29.md) - Historical source for the superseded implementation, calibration, daemon semantics, and the baseline used by the Rust port.

### Lighting, Audio, and Scenes

- [2026-08-31 multimodal/voice handover](./handover-2026-08-31-multimodal-voice.md) - Authoritative wiring, codec, mixer, device ownership, scene schema, physical commissioning, and source-run operational context.

### Voice and Orion Studio

- [2026-09-04](./handover-2026-09-04.md) — Pi voice, startup and attention; physical acceptance pending.

- [2026-08-31 multimodal/voice handover](./handover-2026-08-31-multimodal-voice.md) - Records Chatterbox rejection, Piper Ryan Medium selection, Pi wake/STT implementation and limitations, and the proposed Mac-hosted Studio voice/agent architecture.

### Simulation

- [2026-09-04](./handover-2026-09-04.md) — Pi voice, startup and attention; physical acceptance pending.

- [2026-08-29 Rust handover](./handover-2026-08-29-rust-runtime.md) - Records the Rust-owned MuJoCo backend, Python bridge boundary, integration test, and why Python is optional on the hardware Pi.
- [2026-08-29](./handover-2026-08-29.md) - Aligns MuJoCo with captured physical zero/rest, shared meshes, constrained pose editing, and torque analysis.
- [2026-08-28](./handover-2026-08-28.md) - Records the earlier torque-characterization scope and warns about unrealistic provisional actuator limits.

### ROS 2 History

- [2026-08-29](./handover-2026-08-29.md) - Explains why the active ROS workspace was removed and where useful historical design documents were archived.
- [2026-08-28](./handover-2026-08-28.md) - Historical record of the first `ros2_control` implementation before the architecture changed.

---

## Status Index

### Complete

- [2026-08-31 multimodal/voice handover](./handover-2026-08-31-multimodal-voice.md) - Movement acknowledgement, physical lighting/audio, local scenes, Piper speech, and the functional Pi wake/transcription path are complete.
- [2026-08-29 Rust handover](./handover-2026-08-29-rust-runtime.md) - Rust migration, MuJoCo and physical parity, C++ removal, canonical runtime rename, and current documentation are complete.
- [2026-08-29](./handover-2026-08-29.md) - Recalibration, shared-motion rebuild, native runtime, MuJoCo alignment, physical proof, and ROS removal are complete for this phase.

### In Progress

- [2026-09-04](./handover-2026-09-04.md) — Pi voice, startup and attention; physical acceptance pending.

- [2026-08-31 multimodal/voice handover](./handover-2026-08-31-multimodal-voice.md) - Conversational reliability remains in progress; Studio transport, Mac transcription/wake detection, intent routing, and agent integration are not implemented.
- [2026-08-29 Rust handover](./handover-2026-08-29-rust-runtime.md) - Product continuation is unselected; lighting/audio integration, scene playback, and Orion Studio remain planned.
- [2026-08-28](./handover-2026-08-28.md) - Historical snapshot of an in-progress ROS control plan; its open architecture work was superseded by the 2026-08-29 report.

### Blocked

- None. Further physical validation requires access to the assembled Pi/robot, but the current implementation is operational and the next software work is unblocked.

---

**Index Maintained By:** Orion project handover process
