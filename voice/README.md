# Orion voice

Orion captures microphone audio and detects **“Hey Orion”** on the Pi using
Rustpotter. Studio runs Qwen ASR to confirm the wake phrase and transcribe the
command, sends the command to Codex, and synthesizes the reply with Chatterbox.
The Pi plays the reply through ReSpeaker while `oriond` runs speech animation.

## Setup

From the workstation repository root:

```bash
./scripts/deploy_pi.sh
```

Deployment updates the Pi voice environment, builds the native Rustpotter
adapter, verifies the wake model, and installs the services. It requires an
existing calibrated Pi with the [ReSpeaker driver](../hardware/audio/README.md)
and Rust toolchain. The deployment includes physical movement tests.

Prepare the Mac worker using the [Studio Voice setup](../docs/tutorials/first-studio-voice-run.md).
Pair Studio with Orion once, then enable Orion’s microphone. Pairing is saved
in the desktop credential store; enabling capture remains a separate action.

## Runtime

- Pi service: `orion-listener`, using `voice/.venv` with Python 3.12.
- Audio: 16 kHz mono sent from stereo capture; three seconds of pre-roll in memory.
- Wake model: `models/wake/hey_orion_reference.rpw`, threshold `0.400`.
- Listener: port `7448`, authenticated with `~/.config/orion/studio-token`.
- Studio models: Qwen3-ASR-0.6B and Chatterbox Turbo; no ASR or TTS models are needed on the Pi.

Audio and authentication travel unencrypted over the LAN. Use a trusted network.
Capture opens only while Studio is connected and authenticated. **Stop Orion
microphone** closes capture; Character Stop controls animation separately.

Say “Hey Orion” followed by a request, or pause after the wake phrase and then
speak. Qwen rejects unconfirmed wake candidates before they reach the agent.
Listening resumes after reply playback completes.

## Troubleshooting

On the Pi:

```bash
systemctl is-active oriond orion-studio-gateway orion-listener
journalctl -u orion-listener -u orion-studio-gateway -n 50 --no-pager
```

To repair dependencies from the Pi repository root:

```bash
./scripts/install_pi_voice.sh "$PWD"
sudo systemctl restart orion-listener
```

The installer updates the locked environment in place. If installation fails,
the listener stays stopped; resolve the reported error and rerun the installer.
For playback problems, see [audio troubleshooting](../hardware/audio/README.md).

## Directional attention

Left/right attention is disabled until microphone spacing and channel order
have been measured. Settings live in `~/.config/orion/voice.env`:
`ORION_MIC_SPACING` is the distance in metres and `ORION_CHANNEL_SIGN` is `1`
or `-1`. Both default to zero. Restart the listener after changing them.

Use the [attention setup and constraints](../docs/explanation/voice-attention.md)
and [physical validation procedure](../docs/how-to/validate-character-v2.md)
before enabling directional motion.
