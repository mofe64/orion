# Echo cancellation and barge-in design

Status: proposed. Nothing on this page is implemented. The implemented voice
behaviour is in [voice architecture](voice-architecture.md).

Orion cannot hear a person while it speaks. The listener ignores microphone
audio during processing and playback, then waits for an echo guard before the
follow-up window opens. Acoustic echo cancellation (AEC) would remove Orion's
own voice from the microphone signal. Barge-in would let a person stop a reply
by speaking over it.

This design recommends a staged path: measure the echo on the assembled robot,
add wake-phrase barge-in with a self-trigger veto, then move capture and
playback into one full-duplex audio process with WebRTC AEC3. Open-speech
barge-in stays optional until the measurements support it.

## Terms

- **Near end**: sound from the room, including the person speaking to Orion.
- **Far end** or **reference**: the PCM Orion sends to its speaker.
- **Echo**: the reference as the microphones hear it after the speaker, the
  room and the body change it.
- **Echo path delay**: time from a reference sample entering the playback
  buffer to its echo appearing in capture.
- **ERLE** (echo return loss enhancement): how many decibels of echo the AEC
  removes. Higher is better.
- **Double talk**: the person and Orion speak at the same time. AEC must keep
  the person's speech while removing Orion's.
- **Barge-in**: the person interrupts a reply and Orion stops speaking.

## Constraints from the current system

These facts come from the code and hardware notes. Each one changes the design.

| Fact | Where | Consequence |
| --- | --- | --- |
| Capture and playback run in separate processes. The listener reads `arecord` at 16 kHz stereo; `oriond` writes `aplay` at 24 kHz mono. | [`voice/orion_voice/capture.py`](../voice/orion_voice/capture.py), [`runtime/src/devices/audio.rs`](../runtime/src/devices/audio.rs) | No process holds the reference and the microphone signal together, so no AEC can run today. |
| Both streams use `plughw`, and each reply starts a new `aplay` process with `--buffer-time=100000`. | same files | ALSA resampling and a fresh playback buffer make the echo path delay differ between replies. AEC must estimate delay or receive aligned frames. |
| Cues (chime, alarm) play through separate `aplay` processes from WAV files. | `AlsaAudioDevice::start_playback` | A reference tap on speech PCM alone misses the chime and alarm. |
| One TLV320AIC3104 codec drives both the microphones and the speaker. | [audio hardware](../hardware/audio/README.md) | Capture and playback share one crystal, so the two streams do not drift against each other. Software AEC works well under that condition. |
| The ReSpeaker 2-Mics HAT has no hardware AEC and no loopback channel. | [audio hardware](../hardware/audio/README.md) | The reference must come from software. |
| Codec AGC is off and capture gain is fixed. | `configure-capture.sh` | Good for AEC: automatic gain changes would look like echo path changes. |
| The listener already runs Rustpotter while an alarm sounds and dismisses the alarm on a detection. It sends `session.interrupted`, and the coordinator aborts jobs and cancels playback. | [`voice/orion_voice/satellite.py`](../voice/orion_voice/satellite.py), [`coordinator/src/pipeline.rs`](../coordinator/src/pipeline.rs) | Wake-over-playback detection and a cancel path already exist. Barge-in can reuse them. |
| During `processing` and `playing` the listener drops capture frames. | `SatelliteSession.accept_stereo` | Barge-in needs a new listener phase that keeps Rustpotter running during `playing`. |
| Cancelling an active agent request retires the Codex conversation. | [voice architecture](voice-architecture.md#agent-conversation-and-memory) | Barge-in must stop speech without cancelling a Codex turn that is still streaming, or the person loses context by interrupting. |
| Qwen wake confirmation often arrives only after the full utterance is transcribed. | [voice architecture](voice-architecture.md#confirmed-waking) | Waiting for Qwen before stopping playback would make barge-in feel broken. The stop decision needs a faster signal. |
| Servos move during speech animation. | [character animation](character-animation.md#speech-driven-animation) | Motor noise is near-end noise. It can hold a voice activity detector (VAD) open or look like speech. |
| Replies can contain the word “Orion”. | agent replies | Orion saying its own name can trigger Rustpotter through the speaker. |

Unmeasured facts that also shape the choice:

- Whether the speaker and microphones sit on the same link. If the head moves
  the speaker relative to the HAT, the echo path changes during animation and
  the AEC filter must re-converge.
- The hardware sample rate while both streams are open. Check it on the Pi
  during a reply with
  `cat /proc/asound/card*/pcm0c/sub0/hw_params /proc/asound/card*/pcm0p/sub0/hw_params`.
- Echo level at the microphones relative to a person speaking from 1–2 m. A
  small speaker a few centimetres from the microphones can be 20–30 dB louder
  than the person. That ratio sets how much ERLE is enough.
- Speaker distortion at the `0 dB` PCM target. Linear AEC cannot remove
  distortion; a residual echo suppressor has to.

## Target behaviour

1. While Orion speaks, “Hey Orion” stops the reply within 300 ms of the end of
   the phrase and starts a new command capture.
2. Orion's own speech, including its name, does not stop a reply.
3. Recordings sent to Qwen contain little of Orion's voice or the wake chime.
4. The follow-up window opens without the 0.5 s discard and 300 ms quiet check.
5. Interrupting a reply keeps the Codex conversation, and the agent knows how
   much of its reply the person heard.
6. AEC failure degrades to the implemented behaviour, never to self-interruption.

## AEC options

### Option A: no AEC

Keep the echo in the microphone signal and make detectors tolerate it.

- Cost: none.
- Works for: wake-phrase barge-in if Rustpotter already copes with playback, as
  the alarm path suggests.
- Fails for: clean command audio after an interruption, open-speech barge-in,
  and shortening the echo guard.

### Option B: loopback reference with a delay-estimating AEC

Keep the two processes. Copy every playback stream to an ALSA loopback device
(`snd-aloop`) or a socket, and run AEC in the listener on microphone frames and
the loopback frames.

- Cost: moderate. Cue and speech playback both need the copy, so every `aplay`
  target becomes a `multi` or `dmix` device that includes the loopback.
- Alignment: the loopback frames and the microphone frames have no fixed
  relation. WebRTC AEC3 estimates delays up to several hundred milliseconds,
  but each new `aplay` process restarts the estimate, and the first
  words of each reply leak through while it converges.
- Placement: the listener is Python. AEC3 needs a 10 ms frame every 10 ms;
  garbage collection and `asyncio` stalls add jitter the AEC sees as delay
  changes.

### Option C: one full-duplex audio process with WebRTC AEC3 (recommended end state)

Add one Rust process, `orion-audio`, that opens the card once for capture and
playback with matching period sizes. It owns every sound the speaker makes and
every frame the microphones produce.

```text
oriond  --PCM + cue requests-->  orion-audio  --speaker-->  JST speaker
                                     |   ^
                    reference frame  |   | microphone frame (same period)
                                     v   |
                                 WebRTC AEC3 + noise suppression
                                     |
listener  <--raw stereo + cleaned mono + playback state--+
```

- Alignment: the process writes a playback period and reads the matching
  capture period from one clock. The echo path delay becomes a fixed hardware
  constant that it measures once at startup with a short chirp.
- Reference: covers speech, chime, alarm and scene audio, because all of them
  pass through one mixer.
- AEC engine: WebRTC AEC3 through the `webrtc-audio-processing` library
  (packaged for Debian as `libwebrtc-audio-processing`; confirm the version on
  the Pi OS release before depending on it). It includes a residual echo
  suppressor, which the small speaker needs. SpeexDSP's echo canceller is a
  lighter fallback with weaker double-talk and non-linear handling.
- Cost: largest. `AlsaAudioDevice` gains a socket-backed implementation behind
  the existing `AudioDevice` trait, and `AlsaPcmCapture` gains a socket-backed
  reader. The listener keeps session state; `oriond` keeps playback timing and
  speech animation.
- CPU: AEC3 at 16 kHz mono is expected to use a few percent of one Pi 5 core.
  Measure it beside Qwen and Piper before committing.
- Direction: AEC produces mono. The listener keeps estimating direction from raw
  stereo, which is meaningful only when Orion is silent.

Placing the audio engine inside `oriond` was considered. It would avoid a new
process, but it would move capture into the motor daemon and break the rule that
capture runs independently of the motor loop and survives a runtime restart.

### Option D: PipeWire echo-cancel module

Let PipeWire own the card and run `libpipewire-module-echo-cancel` with the
WebRTC backend.

- Cost: low code, high integration. Orion deliberately excludes this card from
  WirePlumber because WirePlumber reset the mixer; this option reverses that.
- Services run without a desktop session, so PipeWire needs a system or lingering
  user instance with fixed latency settings.
- Debugging moves out of Orion's code into PipeWire configuration.

### Option E: hardware AEC board

Replace the HAT with a microphone array that runs AEC on its own DSP, such as an
XMOS XVF3800 board.

- Cost: new hardware, mount, cabling and direction code. The DSP still needs the
  playback signal routed through it.
- Benefit: no Pi CPU cost and a tuned beamformer.

### Comparison

| Option | ERLE expectation | Reference covers cues | Alignment | Effort | Reversibility |
| --- | --- | --- | --- | --- | --- |
| A | none | n/a | n/a | none | trivial |
| B | medium, weak at reply start | only with extra routing | estimated per reply | medium | easy |
| C | high | yes | fixed by construction | high | medium |
| D | high | yes | handled by PipeWire | medium | medium |
| E | high | yes | handled by DSP | high, hardware | low |

## Barge-in options

### Trigger

| Trigger | Stops on | False interruption risk | Needs AEC |
| --- | --- | --- | --- |
| Wake phrase | “Hey Orion” during a reply | low | no, but AEC improves recall |
| Open speech | any sustained near-end speech | high: echo, motors, TV | yes, with high ERLE |
| Hybrid | duck on speech, stop on wake phrase or a short Qwen check | medium | yes |

Wake-phrase barge-in matches the rest of Orion: the person already addresses
Orion by name, and the alarm path already dismisses on the wake phrase.

### Stop decision

Qwen confirmation is too slow to gate a stop. Choose between:

1. **Stop on a Rustpotter candidate.** Fastest. A false candidate cuts a reply.
2. **Duck on a candidate, stop on a fast verifier.** Lower the reply volume by
   about 12 dB immediately, then stop when a second stage accepts the phrase or
   resume full volume when it rejects it. The openWakeWord verifier under review
   would fit this slot. Without it, a raised Rustpotter threshold during playback
   is the verifier.

Option 2 is recommended. Ducking also raises the near-end to echo ratio for the
command that follows.

### Self-trigger veto

Orion knows exactly what it is saying, so it can check its own output:

- Run a second Rustpotter instance on the reference signal. If the reference
  produced a detection within the echo path delay plus a margin, veto the
  microphone detection.
- The coordinator can also mark reply sentences that contain “Orion” so the
  listener raises its threshold while those samples play.

The reference-side detector is the more reliable check because it uses the
exact audio and timing rather than text.

## Recommended plan

### Stage 0: measure

No product code. Record on the assembled robot:

1. Hardware parameters for both streams while a reply plays.
2. Raw microphone audio while Piper replies play at the `0 dB` target, with and
   without servo motion, and with a person saying “Hey Orion” over the reply
   from 1 m and 2 m.
3. Rustpotter detections on those recordings at the installed threshold and at
   raised thresholds. Count missed barge-ins and self-triggers, including replies
   that say “Orion”.
4. Echo-to-speech level ratio.

A capture helper that tees `arecord` to a file while `oriond` plays a reply is
enough. Keep recordings out of the repository, following the wake-word dataset
rules in [wake-word training](wake-word-training.md).

Decision gate: if raw Rustpotter misses fewer than one in ten barge-ins without
self-triggers, Stage 1 can ship before AEC.

### Stage 1: wake-phrase barge-in with the veto

Behaviour changes:

- The listener keeps Rustpotter running in `playing` with a playback threshold,
  and requires the reference-side veto. Before `orion-audio` exists, `oriond`
  streams the speech PCM it plays to the listener as the veto reference.
- On a detection the listener asks the runtime to duck, then sends
  `session.interrupted` with `reason: "barge_in"` and starts a new wake session
  with the pre-roll it already holds.
- The coordinator stops TTS and playback but lets a streaming Codex turn finish
  and discards its remaining speech, so the conversation survives. It records the
  interruption and the played duration in the turn history.
- The runtime reports how many samples it played. The coordinator maps that to
  the last sentence the person heard and sends it to the agent with the next
  command, for example “Your previous reply was interrupted after: …”.
- The protocol gains a `bargeIn` capability, so older peers keep the implemented
  behaviour.

### Stage 2: `orion-audio` with AEC3

- Introduce the full-duplex process from option C and move both `oriond` playback
  and listener capture onto it.
- Feed Rustpotter, Silero and Qwen the cleaned mono signal; keep raw stereo for
  direction.
- Replace the reference-side veto input with the aligned reference frames.
- Shorten or remove the echo guard once measured residual echo stays below the
  VAD threshold.
- Health check: if the AEC reports divergence or the process restarts, the
  listener disables barge-in and falls back to the echo guard for that reply.

### Stage 3: optional open-speech barge-in

Only if Stage 2 shows high ERLE during servo motion. Use the hybrid trigger:
duck on sustained Silero speech in the cleaned signal, then stop on the wake
phrase or a short Qwen check of that speech.

## Verification

| Check | Method | Target |
| --- | --- | --- |
| ERLE | Play a Piper reply with no near-end speech; compare raw and cleaned level | at least 25 dB after convergence (to be confirmed by Stage 0) |
| Convergence | Level of the first second of cleaned echo per reply | stays below the VAD speech threshold |
| Barge-in recall | Scripted “Hey Orion” over replies at 1 m and 2 m, servos moving | at least 90% |
| Self-interruption | Replies that contain “Orion”, room silent | zero per 100 replies |
| Time to silence | Recorded at the speaker from end of phrase to silence | at most 300 ms |
| Command quality | Qwen transcripts of commands spoken over replies versus silent-room baseline | within 5 points word accuracy |
| CPU | `orion-audio` and listener CPU during a reply beside Qwen and Piper | no added TTS buffering or ASR delay |
| Fallback | Kill `orion-audio` mid-reply | reply stops cleanly; listener reports capture failure and returns to wake detection |

Unit tests cover the listener phase changes, the veto timing window, the
`barge_in` message ordering, and conversation survival in the coordinator.
Acoustic targets need recordings from the assembled robot; simulator and desktop
runs cannot substitute for them.

## Open questions

- Should barge-in stop an alarm reply path the same way, or does alarm dismissal
  stay a separate local action? The design keeps it separate.
- Does the person expect a spoken acknowledgement after an interruption, or only
  the listening light?
- Should Stage 1 wait for the openWakeWord verifier so ducking has a fast
  second stage?
