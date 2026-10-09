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
It keeps pre-roll and the current recording in memory. Mute, coordinator
cancellation and disconnect clear both buffers; acoustic rejection clears the
candidate recording and keeps the rolling pre-roll. Saving wake audio requires
the opt-in
[wake diagnostics](#troubleshooting).

A wake candidate opens a listener/coordinator session. The runtime session,
chime and brief light pulse wait for the acoustic verifier to accept it.
Acoustic rejection ends the candidate immediately and returns to listening,
preserving three seconds of pre-roll, Rustpotter state and verifier history.
The listener cancels runtime feedback for the rejected ID; the coordinator
clears that session and sends its cancellation control. Held audio is discarded
without ASR. A loaded verifier remains authoritative even when marked unhealthy;
that speed guard disables playback barge-in. If no verifier is loaded,
a short wake prefix reaches Qwen while full command capture continues.
Prefix verification and transcription of the complete utterance run in order. Follow-up
speech can remain buffered during confirmation. The coordinator rejects recordings
that reach the capture limit before submitting a command to the agent.

The listener reports when recording ends with `silence` or `max_duration`, including
capture time. These log entries contain no audio or transcript. See
[capture and session behavior](../docs/voice-architecture.md#capture-ownership-and-session-lifecycle)
for timing, limits and the Silero classifier.

Processing suppresses wake detections. On V2, playback keeps three seconds of
pre-roll and runs Rustpotter only while the acoustic phrase verifier is healthy.
A candidate leaves playback running until the verifier accepts it. Acceptance
stops the reply and starts a fresh verified wake session from that pre-roll;
rejection leaves the speaking session unchanged. A missing or unhealthy verifier
disables barge-in. V1 retains playback wake suppression.

Barge-in uses processed channel 0 and the XVF3800's hardware echo cancellation;
Orion does not add software AEC or raise the wake threshold during playback.
Only “Hey Orion” interrupts, and interruption sessions never request an attention
turn because Orion's voice can steer the board's beam. Alarm dismissal takes
precedence. After successful playback, the listener still establishes a quiet
baseline and opens the teal follow-up invitation.

## Settings and microphone startup

`--hardware v1` uses HAT capture routing. `--hardware v2` uses USB capture
without HAT mixer commands; `--capture-channels 2|6` and `--processed-channel N`
select the verified XVF3800 stream. The processed channel feeds the listener
without averaging raw microphone channels. V2 attention uses calibrated USB beam
observations when explicitly enabled; it defaults off. See [USB profile setup](../docs/hardware-versions.md#usb-capture-and-playback).

Microphone mute persists in `~/.config/orion/microphone.json`. The listener applies
capture routing before opening ALSA, discards startup frames, reapplies gain after
the ADC starts, and then reports readiness. Unmute can therefore take a short
startup interval before capture is ready.

Set microphone overrides in `~/.config/orion/voice.env` and restart the listener.
The [configuration reference](../docs/configuration.md#pi-runtime-and-listener)
lists wake sensitivity, gain, VAD and direction settings. Direction estimation
on V1 requires measured microphone spacing and channel orientation. V2 requires
the measured mounting offset and sign; both default to unset.

## Troubleshooting

On the Pi:

```bash
systemctl status orion-listener orion-voice-stack
journalctl -u orion-listener -u orion-voice-stack -n 60 --no-pager
```

To diagnose live acoustic-verifier rejection, set this in
`~/.config/orion/voice.env`, then restart the listener after deploying the change:

```bash
ORION_WAKE_DEBUG_DIR=/home/mofe/orion-wake-debug
```

The variable defaults unset: no diagnostic writer, audio files or additional
score buffer are created. When enabled, each accepted or rejected acoustic verdict
saves a `<UTC timestamp>-accepted/` or `-rejected/` folder containing `audio.wav` and
`meta.json`. The WAV holds up to four seconds of exact mono verifier input at
16 kHz, ending when the verdict is produced. Metadata records the session ID and
phase at the candidate, Rustpotter score, candidate/verdict sample positions,
all verifier chunk scores in that window (including ineligible warm-up scores),
verifier health, its last 250 processing times in seconds, and time since capture
last opened. `audio_start_sample` and `audio_end_sample` map WAV samples to the
verifier's sample positions; capture reopening resets that origin. Alarm
dismissal does not save a recording.

A bounded queue writes on a separate thread and keeps the newest 50 diagnostic
folders. A full queue drops snapshots and logs a warning; disk errors also log
without changing wake decisions. Once a minute during capture,
`voice.capture_read_timing` reports `reads_over_40_ms`. Read latency includes
dispatch to and from the capture worker thread, so CPU scheduling delays count.

The audio stays on the Pi; it is not uploaded or included in Studio history.
Unset the variable and restart the listener after testing, then delete the saved
audio and metadata after use. For the V2 check, make five wakes from idle and
three from the follow-up window, then replay each WAV through `StreamingScorer`
on the Pi. High offline scores paired with low live scores point toward verifier
state or timing. Low scores in both point toward the live audio: compare it by
ear and spectrum with the measurement recordings. These diagnostics do not change
wake thresholds, models or verification rules.

For dependency repair, prepare and deploy a complete replacement release through
the [Pi deployment procedure](../docs/quickstart.md#deploy-to-the-pi). It builds the
Python environment before switching service paths. Playback routing and speaker
checks are described in [audio setup](../hardware/audio/README.md).

If wake detection succeeds but the body stays at rest, inspect
`voice.wake_verifier` acceptance and the runtime's rest status. With
`--no-verifier`, inspect Qwen confirmation: the candidate chime can play even
when Qwen rejects the phrase. Home movement requires confirmation and a healthy
rest lifecycle. If a recording
ends early, inspect the VAD configuration, capture gain and endpoint reason before
changing the ASR model.

## Intermediate tool speech

The listener advertises `toolFeedback: true`. After search acknowledgement plays,
`session.processing` restores thinking feedback in the same session. Wake
detection stays suppressed during processing. V2 playback accepts verified
barge-in, and alarm dismissal takes precedence. During synthesis and playback,
`session.keepalive` renews the
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

## Check V2 barge-in on the lamp

From 1–2 m, interrupt ten long replies with “Hey Orion” and a new command: at
least nine must stop and capture the command. Play fifty uninterrupted replies,
including replies saying “Orion”: none may interrupt themselves. After an
interruption, ask “what were you saying?” to check retained conversation context.
The [coordinator lifecycle](../docs/voice-architecture.md#wake-phrase-barge-in)
explains why the new command can wait for the old agent turn to finish.

## Calibrate V2 attention

Keep processed channel 0. Fit the mounting with the base cover in its normal
position: from about 1.5 m, record `dir-front`, `dir-left` and `dir-right` trials
with `--expected-azimuth 0`, `90` and `270`, saying “Hey Orion” three times at
each position. Run `analyze` as above and copy its mounting `sign` and
`offset_deg` into the [XVF listener settings](../docs/configuration.md#pi-runtime-and-listener).
Set `ORION_XVF_DIRECTION=1` only after the positions separate clearly. If the
cover collapses the positions into two beams, report that acoustic obstruction
before enabling facing.

The poller reads auto-select beam 3 at 20 Hz without blocking capture. It converts
board radians to lamp degrees with `wrap180(sign * (board_deg - offset_deg))`:
positive is counter-clockwise from front seen from above, toward the lamp's left.
The default front sector is ±30 degrees; other angles vote for their nearer side,
including angles behind the lamp. A turn needs at least five speech-energy votes
in three seconds with 75% agreement. Centre/unknown send no attention command.

Before acceptance, verify that `attention_left` physically points left; V2
attention poses still require hardware playback validation. Report swapped poses
to the deploy smoke-test owner. Try five wakes from each side and front: at least
12 of 15 must face correctly or remain front, with no wrong-way turns. Board
access/read failures log once and leave voice working without a turn.

## Validation

With a prepared listener environment, run from the repository root:

```bash
PYTHONPATH=voice voice/.venv/bin/python -m unittest discover -s voice/tests -v
```

Tests cover endpointing, retained command audio, mute, disconnects, session
ordering and transport failures. Physical checks establish capture quality,
speaker pickup and direction behavior on the assembled robot.
