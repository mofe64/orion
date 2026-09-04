# Orion voice architecture

The Raspberry Pi owns Orion's microphone and Rustpotter wake detector. Studio
owns Qwen3-ASR, the configured agent and Chatterbox synthesis. Studio never opens
its workstation microphone or loads a wake detector.

## Audio and control flow

```text
Pi ReSpeaker stereo capture (16 kHz signed PCM16)
  -> local coarse direction observation before downmixing
  -> mono Rustpotter + three-second in-memory pre-roll
  -> wake candidate notification over token-authenticated WebSocket
  -> Pi speech endpoint -> bounded complete utterance upload
  -> Studio Qwen ASR confirms "Hey Orion" and extracts command
  -> configured AgentProvider -> Chatterbox Turbo
  -> existing validated WAV upload -> oriond-owned playback
  -> speech animation -> terminal playback acknowledgement -> rearm Pi
```

Rustpotter is the only active wake detector. Its reference and native adapter
live under `voice/`; Studio does not depend on that package. Qwen retains the
second-stage false-positive check. Rejected wake candidates never reach the
agent or trigger directional attention. Qwen confirmation is not a guarantee
that every false positive is eliminated.

## Capture ownership and session lifecycle

The Pi listener is an independent process, outside the 50 Hz motor loop. The
service may run at boot, but opens capture only while an authenticated Studio
owner has enabled the Orion microphone. Closing Voice or disconnecting ends
capture. A single connection owns capture; additional connections are refused.

Every interaction has a random session ID. The Pi captures continuously while
enabled, keeps three seconds of pre-roll in memory and sends a candidate event
immediately. Audio is uploaded as a complete endpointed utterance, not a
continuous room-audio stream. An utterance is at most 18 seconds including
pre-roll. Endpointing uses the existing deterministic energy detector: at least
1.2 seconds of capture, 800 ms of trailing silence and a 15-second maximum.
These workstation-origin thresholds require evaluation on the ReSpeaker.

If the first transcript is only the wake phrase, Studio requests a follow-up
command. The Pi buffers up to one endpointed follow-up while Qwen is working,
so the transition does not discard speech spoken during confirmation. Empty
commands fail without invoking the agent.

Processing suppresses further wake triggers. Playback acknowledgement is sent
only after the Pi reports terminal playback. This is turn-taking, not acoustic
echo cancellation or barge-in. Session deadlines, bounded socket queues and
strict state transitions prevent indefinite buffering and stale command replay.
A disconnected session is discarded; the operator re-enables Voice to reconnect.
Processing has a 120-second session lease; entering playback grants 180 seconds.

## Transport and deployment

Studio saves the paired gateway address and token in the OS credential store.
Gateway reconnect restores read-only status/authoring connectivity without
replaying robot operations or enabling Voice. The listener token is reused from
that saved connection. See [pairing configuration](../reference/configuration.md#saved-pairing).

The Pi listener uses plain WebSockets on port 7448 and the Pi's existing
Studio token for authentication. Studio derives `ws://GATEWAY_HOST:7448/`
from the gateway connection; `ORION_PI_VOICE_URL` can override it with another
`ws://` endpoint. No certificate setup is required. Tokens and audio travel
unencrypted, so this development connection is intended for a trusted LAN.
Token authentication controls access but does not protect against network
interception.

Studio's native launcher passes connection configuration to its Python worker.
The worker connects directly to the Pi listener and exposes processing events
to the UI through its existing authenticated loopback socket. Version 5 of that
local protocol rejects workstation microphone frames. Pi protocol version 1
uses JSON session messages and length-checked PCM16 utterances.

The existing HTTP gateway still handles response WAV upload and robot control;
both transports are unencrypted. Production pairing and encryption for voice
and gateway transport remain separate work.

Follow [Pi voice setup](../../voice/README.md) and the
[Studio Voice tutorial](../tutorials/first-studio-voice-run.md).

## Agent and physical boundary

Raw microphone audio travels only between the Pi and Studio. Qwen and
Chatterbox run on the workstation. With the Codex provider, confirmed command
text is sent to the configured Codex service; audio is not. The agent produces
spoken replies and has no motion or device command capability.

The Pi may request allowlisted character reactions based on session events.
Confirmed, confident direction observations request the existing runtime's
semantic attention operation. Character Off prevents those movements while
voice can remain enabled. See [Voice attention](voice-attention.md) for the
animation brief, priority, commissioning and acceptance requirements.

## Legacy offline diagnostics

Sherpa, Moonshine and Piper remain available only through the optional
`fallback` dependency set and explicit legacy diagnostic commands. They are
not started by `orion-listener` or Studio. Never run a legacy microphone worker
concurrently with the primary listener. Piper remains usable for Pi-local TTS.
