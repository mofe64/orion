# Orion voice architecture

Orion has two voice paths with different roles. Studio Voice is the primary
interactive path. The Raspberry Pi-local stack remains an offline fallback and
diagnostic path.

## Primary path: Studio Voice

```text
workstation microphone
  -> browser AudioWorklet
  -> native-rate conversion to 16 kHz mono PCM16
  -> authenticated loopback WebSocket
  -> Rustpotter reference wake detector
  -> endpointing and in-memory pre-roll
  -> Qwen3-ASR wake confirmation and command transcription
  -> configured AgentProvider
  -> Chatterbox Turbo speech generation
  -> authenticated mono PCM16/24 kHz WAV upload
  -> oriond-owned ReSpeaker playback
  -> energy-driven character motion and RGBW light
```

Studio begins capture only after the user selects **Enable microphone**. Raw
audio, endpoint buffers, and the three-second pre-roll remain transient in
memory. The WebSocket accepts loopback connections only and uses a fresh token
for every worker process.

Rustpotter is deliberately a fast candidate detector rather than the final
authority. Qwen3-ASR confirms that the transcript begins with “Hey Orion”
before the agent receives the command. This two-stage design allows a
relatively permissive reference threshold without sending every sound to ASR
or the agent.

The current agent produces a spoken response only. It cannot request movement,
lighting, cues, or scenes. Studio maps known voice-pipeline phases to the
allowlisted listening, thinking, and neutral character states; agent-generated
text never becomes a raw robot command.

Once a validated response reaches `oriond`, waveform energy and phrase peaks
drive one utterance-length head-led performance. That animation path is
documented in [Character animation design](character-animation.md#speech-driven-animation).

## Agent and privacy boundary

Microphone audio, wake detection, transcription, and speech generation remain
on the workstation with the default model adapters. The configured agent is a
separate boundary:

- With the current Codex provider, the confirmed text command is sent to the
  configured Codex service. Raw microphone audio is not sent.
- A future local-LLM provider can keep confirmed text local without changing
  wake detection, ASR, TTS, or playback.
- An OpenAI Platform provider can use the same `AgentProvider` contract with
  separately configured credentials.

Cloud-backed agent use is therefore optional application behaviour, not a
requirement for the Pi runtime, scenes, or manual Studio control. The Voice UI
must continue to disclose the selected provider before capture is enabled.

## Model lifecycle

Voice model weights are not stored in Git or bundled with a source checkout.
The `orion-voice-models` command pre-downloads and verifies the configured ASR
and TTS repositories in the user's Hugging Face cache.

Starting Studio alone does not proactively download the weights. If Voice is
enabled without the prefetch step, the model libraries may try to download
them during worker startup. That first load can exceed Studio's startup
timeout and is not the supported first-run path. Follow the
[Studio Voice tutorial](../tutorials/first-studio-voice-run.md) on every new
development device.

See [model management](../how-to/manage-studio-voice-models.md) for cache
locations, overrides, and offline preparation.

## Fallback path: Raspberry Pi-local voice

```text
ReSpeaker microphones
  -> Sherpa keyword detection
  -> Silero VAD
  -> Moonshine Tiny English transcription
  -> /tmp/orion-wake.sock transcript events

text request
  -> /tmp/orion-tts.sock
  -> Piper Ryan Medium
  -> temporary WAV
  -> oriond-owned ReSpeaker playback
```

This path performs wake detection and speech-to-text locally on the Pi, but it
stops at transcript publication. It does not interpret commands or invoke an
agent. Piper speech generation is integrated with `oriond` so playback retains
normal run IDs, cancellation, and device ownership.

The [Pi-local voice setup](../../voice/README.md) uses a model installer that is
independent of the Studio model downloader.

## Why both paths exist

The workstation can run more capable ASR and TTS models and usually provides a
better microphone. The Pi path keeps hardware diagnosis and offline operation
possible without making the lower-powered Pi the quality ceiling for the main
experience.
