# Orion voice listener

The Pi listener captures microphone audio, detects “Hey Orion” with Rustpotter,
confirms it with an openWakeWord phrase verifier, and uses Silero to find the
end of speech. It passes recordings to the onboard
coordinator and forwards voice-session events to `oriond` for acknowledgement,
waking and character feedback.

## Setup

Use [Pi installation and deployment](../docs/quickstart.md#prepare-a-new-pi)
to prepare the listener with the complete voice stack. The Pi needs working
[ReSpeaker audio](../hardware/audio/README.md), calibration and a Rust toolchain.
Deployment builds the native Rustpotter adapter and checks the active
reference model, retained trained model and packaged phrase verifier. See [wake-word training](../docs/wake-word-training.md)
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

A wake candidate registers a runtime session. The chime and brief light pulse
wait for the acoustic verifier to accept it. If the verifier is unavailable,
a short wake prefix reaches Qwen while full command capture continues.
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

`--hardware v1` uses HAT capture routing. `--hardware v2` uses USB capture
without HAT mixer commands; `--capture-channels 2|6` and `--processed-channel N`
select the verified XVF3800 stream. The processed channel feeds the listener
without averaging raw microphone channels, and direction-based attention is
disabled. See [USB profile setup](../docs/hardware-versions.md#usb-capture-and-playback).

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

## Intermediate tool speech

The listener advertises `toolFeedback: true`. After search acknowledgement plays,
`session.processing` restores thinking feedback in the same session. Wake detection
and command dispatch stay suppressed while the agent works, except for alarm
dismissal. During synthesis and playback, `session.keepalive` renews the
180-second lease and forwards `voice SESSION keepalive` to the runtime without
changing the phase or triggering feedback. Unknown and expired sessions cannot
renew. `session.processing` and `session.playing` mark actual phase transitions. A long response can continue while the
coordinator owns it; a disappeared owner still expires. Final `session.finish`
starts the echo guard and follow-up invitation. See [agent conversation](../docs/voice-architecture.md#agent-conversation-and-memory).

## Alarm dismissal

The listener polls `routines status` over the runtime socket every 200 ms. A ringing
alert interrupts an existing voice session and enables Rustpotter detection during
playback. “Hey Orion” sends a local stop request and consumes that wake phrase.
It works without a connected coordinator and does not send alarm audio to ASR.
Microphone mute still closes capture. The runtime enforces the five-minute sound
limit independently.

## Measure the XVF3800 on V2

`orion_voice.xvf_measure` records every USB capture channel while it polls the
board's beam azimuths, per-beam speech energy and AEC convergence over the USB
control interface (vendor `2886`, product `001a`). With `--play` it plays a WAV
through the same card, so the board's echo canceller receives it as the far end,
exactly as `oriond` playback does. It only reads board parameters; it never
changes them.

One-time access to the control interface for the service user (log out and in
again if `plugdev` was just added):

```bash
echo 'SUBSYSTEM=="usb", ATTR{idVendor}=="2886", ATTR{idProduct}=="001a", MODE="0660", GROUP="plugdev"' \
  | sudo tee /etc/udev/rules.d/60-orion-xvf3800.rules
sudo udevadm control --reload && sudo udevadm trigger
groups | grep -w plugdev
```

The listener holds the capture device, so stop it for the session and start it
again afterwards:

```bash
sudo systemctl stop orion-listener
# ... trials ...
sudo systemctl start orion-listener
```

Make a reply WAV in Orion's voice. Include “Orion” so self-triggers show up:

```bash
ORION_PIPER_MODEL_DIR=$HOME/.local/share/orion/voice-stack/models/piper-alba-medium \
PYTHONPATH=speech speech/.venv/bin/python - <<'PY'
import wave
from orion_speech_worker.piper import PiperAlbaSynthesizer
text = ("Orion here. The kettle takes about three minutes, so you have time to find a mug. "
        "If you want, Orion can set a timer while you wait, and remind you when it is done.")
pcm = b"".join(chunk.pcm for chunk in PiperAlbaSynthesizer("piper-alba-medium").stream(text))
with wave.open("/tmp/orion-reply.wav", "wb") as out:
    out.setnchannels(1); out.setsampwidth(2); out.setframerate(24000); out.writeframes(pcm)
PY
```

The tool needs `pyusb`, which the voice environment gains on the next deploy.
Run trials from the `voice` folder. Trials land in `~/orion-measurements/`:

```bash
cd voice
M=".venv/bin/python -m orion_voice.xvf_measure"
$M record --label echo-quiet --play /tmp/orion-reply.wav          # nobody speaks
$M record --label echo-moving --play /tmp/orion-reply.wav                       # run a motion from Studio meanwhile
$M record --label barge-1m --play /tmp/orion-reply.wav --expected-azimuth 0     # say “Hey Orion” twice over the reply from 1 m
$M record --label barge-2m-moving --play /tmp/orion-reply.wav --expected-azimuth 0
$M record --label dir-090 --seconds 8 --expected-azimuth 90                     # say “Hey Orion” three times from that position
```

`--expected-azimuth` is the speaker's position in degrees, counter-clockwise from
the lamp's front seen from above. Repeat the direction trial every 45° (0, 45, …,
315) at about 1.5 m. Then summarise everything:

```bash
$M analyze ~/orion-measurements/* --json ~/orion-measurements/summary.json
```

The summary reports, per trial, the level of channel 0 (conference) and channel
1 (ASR) before, during and after playback, how quickly `AEC_AECCONVERGED` reached
1, Rustpotter hits per channel (marked when they fall inside playback) at the
service threshold of 0.35, and the auto-select beam's azimuth while the board
reports speech. Across direction trials it fits the board's mounting offset and
rotation sense, and prints the worst error after that fit. With the six-channel
firmware it also reports echo reduction against the raw microphones.

## Validation

With a prepared listener environment, run from the repository root:

```bash
PYTHONPATH=voice voice/.venv/bin/python -m unittest discover -s voice/tests -v
```

Tests cover endpointing, retained command audio, mute, disconnects, session
ordering and transport failures. Physical checks establish capture quality,
speaker pickup and direction behavior on the assembled robot.
