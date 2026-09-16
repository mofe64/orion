# Orion voice

Orion captures microphone audio and detects **“Hey Orion”** on the Pi with
Rustpotter. Silero decides when speech has ended. The onboard coordinator sends
complete audio to Qwen, confirms the wake phrase, invokes Codex, and synthesizes
Pocket speech. `oriond` owns speaker playback and speech animation. Studio is
an optional settings and observation client.

## Setup

From the workstation repository root:

```bash
./scripts/deploy_pi.sh
```

Deployment updates the Pi voice environment, builds the native Rustpotter
adapter, verifies the wake model, and installs the services. It requires an
existing calibrated Pi with the [ReSpeaker driver](../hardware/audio/README.md)
and Rust toolchain. The service switch returns the robot to rest; normal runtime
startup may move it home. Deployment does not run expression smoke tests.

For the Pi speech and agent service, follow [Pi installation](../docs/quickstart.md#pi-local-voice-and-agent).
Pi capture defaults on unless explicitly muted. Closing Studio leaves the Pi
voice stack running. The agent still requires internet access.

## Runtime

- Pi service: `orion-listener`, using `voice/.venv` with Python 3.12.
- Audio: 16 kHz mono sent from stereo capture; three seconds of pre-roll in memory.
- Wake model: `models/wake/hey_orion_reference.rpw`, threshold `0.400`.
- Listener: port `7448`, authenticated with `~/.config/orion/studio-token`.
- Onboard speech: Qwen3-ASR-0.6B GGUF and Pocket TTS, managed by `orion-voice-stack`.

Onboard processing uses loopback connections. Remote Studio controls use the
authenticated gateway over the trusted LAN.
Capture opens with the listener service and survives processing disconnects.
**Mute Orion microphone** closes capture, clears buffered audio and saves mute
across restarts; Character Stop controls animation separately.

[Automatic rest](../docs/system-architecture.md#automatic-rest-and-waking) leaves capture running.
Deploy the listener and runtime together: every ASR-confirmed wake is forwarded
to `oriond`, even without a usable microphone direction.

Say “Hey Orion” followed by a request, or pause after the wake phrase and then
speak. Qwen rejects unconfirmed wake candidates before they reach the agent.
After a successful reply, wait for the soft teal pulse and continue without
"Hey Orion". The invitation closes after five seconds without speech; the next
request then needs the wake phrase. A silent echo guard precedes the pulse.
See [conversation timing and limitations](../docs/voice-architecture.md#capture-ownership-and-session-lifecycle).

The managed listener uses Silero with a fixed 2× gain in the VAD branch and
1.2 seconds of trailing silence. Captured audio remains unchanged. See the [endpoint rules](../docs/voice-architecture.md#capture-ownership-and-session-lifecycle).
Allow a short quiet interval after enabling capture before the first wake.
The listener logs `voice.endpoint` with capture time,
and `silence` or `max_duration` reason; it does not log audio or transcripts.

## Upgrade from legacy Pi voice

If this checkout previously ran the Sherpa, Moonshine, or Piper workers, stop
and retire those workers before updating their environment. On the Pi, from
the repository root:

```bash
python3 scripts/retire_pi_voice.py "$PWD" \
  --backup "$HOME/.local/share/orion/backups/legacy-voice-$(date +%Y%m%d-%H%M%S)"
```

The migration stops legacy workers belonging to this checkout and archives
recognized legacy models. It preserves the Rustpotter reference. Then use the
normal deployment or repair procedure. The listener installer does not run
this one-time migration automatically.

## Troubleshooting

On the Pi:

```bash
systemctl is-active oriond orion-studio-gateway orion-listener
journalctl -u orion-listener -u orion-studio-gateway -n 50 --no-pager
```

To repair onboard dependencies, use the [full Pi deployment](../docs/quickstart.md#deploy-to-the-pi).
It prepares replacement environments before switching services and preserves
saved settings. The standalone `install_pi_voice.sh` is only for older
listener-only installations; it refuses to update an installed onboard stack.
For playback problems, see [audio troubleshooting](../hardware/audio/README.md).

## Directional attention

Left/right attention is disabled until microphone spacing and channel order
have been measured. Settings live in `~/.config/orion/voice.env`:
`ORION_MIC_SPACING` is the distance in metres and `ORION_CHANNEL_SIGN` is `1`
or `-1`. Both default to zero. Restart the listener after changing them.

Review the [direction evidence and constraints](../docs/voice-architecture.md#direction-evidence)
and [runtime attention behavior](../runtime/README.md#character-startup-and-voice-attention)
before enabling directional motion.

## Intermediate tool speech

The listener advertises `toolFeedback: true`. After search acknowledgement
playback, `session.processing` returns the same session to processing with
capture suppressed. It asks `oriond` to restore thinking animation and breathing
light without another cue. Only final `session.finish` starts the echo guard
and follow-up invitation. Deploy this listener together with the matching
runtime to support the `processing` voice-feedback event.
