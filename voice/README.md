# Orion voice listener

The Pi listener captures microphone audio, detects “Hey Orion” with Rustpotter,
and uses Silero to find the end of speech. It passes recordings to the onboard
coordinator and forwards voice-session events to `oriond` for acknowledgement,
waking and character feedback.

## Setup

Use [Pi installation and deployment](../docs/quickstart.md#pi-local-voice-and-agent)
to prepare the listener with the complete voice stack. The Pi needs working
[ReSpeaker audio](../hardware/audio/README.md), calibration and a Rust toolchain.
Deployment builds the native Rustpotter adapter and checks both the active
trained model and retained reference. See [wake-word training](../docs/wake-word-training.md)
for the evidence and rollback policy.

The listener runs as `orion-listener` with a Python 3.12 environment inside the
active release. Its WebSocket endpoint uses port 7448 and the Pi's
`~/.config/orion/studio-token`. The managed `--local-processor` setting reserves
processing ownership for a loopback connection. Separate authenticated controls
can inspect or change microphone mute.

## Capture and sessions

The listener opens synchronized stereo PCM16 capture at 16 kHz. It estimates
coarse direction from stereo frames and downmixes to mono for Rustpotter and ASR.
It keeps pre-roll and the current recording in memory; recordings are cleared on
mute, cancellation or disconnect.

A wake candidate registers a runtime session and starts the chime and brief
light pulse. With a compatible
coordinator, a short wake prefix reaches Qwen while full command capture continues.
Prefix verification and transcription of the complete utterance run in order. Follow-up
speech can remain buffered during confirmation. The coordinator rejects recordings
that reach the capture limit before submitting a command to the agent.

The listener reports when recording ends with `silence` or `max_duration`, including
capture time. These log entries contain no audio or transcript. See
[capture and session behavior](../docs/voice-architecture.md#capture-ownership-and-session-lifecycle)
for timing, limits and the Silero classifier.

Processing and playback suppress new wake detections. After the coordinator
acknowledges successful playback, the listener establishes a quiet baseline and
opens the teal follow-up invitation. Acoustic echo cancellation and interruption
during playback are not implemented.

## Settings and microphone startup

Microphone mute persists in `~/.config/orion/microphone.json`. The listener applies
capture routing before opening ALSA, discards startup frames, reapplies gain after
the ADC starts, and then reports readiness. Unmute can therefore take a short
startup interval before capture is ready.

Set microphone overrides in `~/.config/orion/voice.env` and restart the listener.
The [configuration reference](../docs/configuration.md#pi-runtime-and-listener)
lists wake sensitivity, gain, VAD and direction settings. Direction estimation
requires measured microphone spacing and channel orientation; its default settings
disable attention turns.

## Troubleshooting

On the Pi:

```bash
systemctl status orion-listener orion-voice-stack
journalctl -u orion-listener -u orion-voice-stack -n 60 --no-pager
```

For dependency repair, prepare and deploy a complete replacement release through
the [Pi deployment procedure](../docs/quickstart.md#deploy-to-the-pi). It builds the
Python environment before switching service paths. Playback routing and speaker
checks are described in [audio setup](../hardware/audio/README.md).

If wake detection succeeds but the body stays at rest, inspect Qwen confirmation
and the runtime's rest status. The candidate chime can play even when Qwen
rejects the phrase; home movement still requires confirmation and a healthy
rest lifecycle. If a recording
ends early, inspect the VAD configuration, capture gain and endpoint reason before
changing the ASR model.

## Upgrade from legacy Pi voice

Older checkouts may contain Sherpa or Moonshine workers. The retirement
helper stops matching legacy workers and archives recognized models while keeping
the Rustpotter reference. Run it only when migrating one of those installations:

```bash
python3 scripts/retire_pi_voice.py "$PWD" \
  --backup "$HOME/.local/share/orion/backups/legacy-voice-$(date +%Y%m%d-%H%M%S)"
```

The standalone `scripts/install_pi_voice.sh` supports older listener-only setups.
It refuses to update an installed onboard voice stack. Ordinary full-stack
updates use the release deployment path.

## Intermediate tool speech

The listener advertises `toolFeedback: true`. After search acknowledgement plays,
`session.processing` restores thinking feedback in the same session. Wake detection
and command dispatch stay suppressed while the agent works, except for alarm
dismissal. Final `session.finish` starts the echo guard and
follow-up invitation. See [agent conversation](../docs/voice-architecture.md#agent-conversation-and-memory).

## Alarm dismissal

The listener polls `routines status` over the runtime socket every 200 ms. A ringing
alert interrupts an existing voice session and enables Rustpotter detection during
playback. “Hey Orion” sends a local stop request and consumes that wake phrase.
It works without a connected coordinator and does not send alarm audio to ASR.
Microphone mute still closes capture. The runtime enforces the five-minute sound
limit independently.

## Validation

With a prepared listener environment, run from the repository root:

```bash
PYTHONPATH=voice voice/.venv/bin/python -m unittest discover -s voice/tests -v
```

Tests cover endpointing, retained command audio, mute, disconnects, session
ordering and transport failures. Physical checks establish capture quality,
speaker pickup and direction behavior on the assembled robot.
