# 0003: Host the primary voice pipeline in Studio

- **Status:** Accepted
- **Scope:** Wake detection, ASR, agent response, TTS, and user audio devices

## Context

The Raspberry Pi can run a compact offline voice stack, but its compute budget
and ReSpeaker input quality limit the models and experience. The workstation
has a stronger microphone, more compute, and direct user control over capture.

## Decision

Use Studio as the primary interactive voice host. Capture the workstation
microphone only after explicit user action, stream transient PCM over an
authenticated loopback connection, confirm Rustpotter candidates with
Qwen3-ASR, isolate the agent behind `AgentProvider`, generate speech with
Chatterbox, and play it through the workstation speaker.

Keep all Pi physical capability execution behind the gateway and `oriond`.
The initial agent produces speech only.

## Consequences

- Orion can use models that do not fit comfortably on the Pi.
- Raw microphone audio does not need to cross the network to the robot or a
  cloud agent.
- The current MLX implementation limits full local inference to Apple Silicon.
- A new development device must install the worker and prefetch model weights.
- Packaged applications require a deliberate Python/model distribution design.

