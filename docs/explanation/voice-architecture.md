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
  -> ordered WAV chunk uploads -> one oriond-owned streaming player
  -> speech animation -> terminal playback acknowledgement -> rearm Pi
```

Rustpotter is the only active wake detector. Its reference and native adapter
live under `voice/`; Studio does not depend on that package. Qwen retains the
second-stage false-positive check. Rejected wake candidates never reach the
agent or trigger directional attention. Qwen confirmation is not a guarantee
that every false positive is eliminated.

## Capture ownership and session lifecycle

The Pi listener is an independent process, outside the 50 Hz motor loop. The
service opens capture when it starts unless a saved mute preference disables it.
Closing Studio or disconnecting its processing station leaves capture running.
One authenticated connection owns processing; additional processing connections
are refused. Separate authenticated control connections can inspect or change
mute. Muting closes capture, clears buffered audio and persists across restarts.

Every interaction has a random session ID. The Pi captures continuously while
enabled, keeps three seconds of pre-roll in memory and sends a candidate event
immediately. Audio is uploaded as a complete endpointed utterance, not a
continuous room-audio stream. An utterance is at most 18 seconds including
pre-roll. Endpointing uses DC-corrected energy for its decisions; the audio
sent to Rustpotter and Studio remains unchanged. While listening, it retains
six seconds of frame energies. At wake detection it excludes the latest second,
takes the lower quartile of the remaining window, and freezes a threshold of
three times that energy, with a floor of 500 PCM units. With less than half a
second of eligible history it uses the floor. Allow at least 1.5 seconds of
quiet listening after enabling capture to establish a background estimate.

The same frozen threshold applies to wake capture and a buffered follow-up.
A sustained 60 ms above threshold resets the silence timer; isolated shorter
spikes do not. Capture ends after one second of non-speech, subject to a
1.2-second minimum and a 15-second maximum. Noise estimation resumes only
when listening for a new wake, so response playback cannot raise the threshold.
These energy rules are a prototype based on Pi measurements; clipping,
continuous background speech, and quiet commands still require physical evaluation.

If the first transcript is only the wake phrase, Studio requests a follow-up
command. The Pi buffers up to one endpointed follow-up while Qwen is working,
so the transition does not discard speech spoken during confirmation. Empty
commands fail without invoking the agent.

Processing suppresses further wake triggers. Playback acknowledgement is sent
only after the Pi reports terminal playback. This is turn-taking, not acoustic
echo cancellation or barge-in. Session deadlines, bounded socket queues and
strict state transitions prevent indefinite buffering and stale command replay.
A disconnected session is discarded; the Studio worker reconnects automatically.
Processing has a 120-second session lease; entering playback grants 180 seconds.

## Transport and deployment

Studio saves the paired gateway address and token in the OS credential store.
Gateway reconnect restores status/authoring connectivity without replaying robot
operations. Paired Studio starts its own voice worker while the app is open. The listener token is reused from
that saved connection. See [pairing configuration](../reference/configuration.md#saved-pairing).

The Pi listener uses plain WebSockets on port 7448 and the Pi's existing
Studio token for authentication. Studio derives `ws://GATEWAY_HOST:7448/`
from the gateway connection; `ORION_PI_VOICE_URL` can override it with another
`ws://` endpoint. No certificate setup is required. Tokens and audio travel
unencrypted, so this development connection is intended for a trusted LAN.
Token authentication controls access but does not protect against network
interception.

Studio's native launcher owns one voice-worker child and stops it when the app
exits. Closing the Voice panel only detaches its status connection. Credentials
reach the worker through its parent pipe and are not saved in a service file.
The worker connects directly to the Pi and exposes status through an authenticated
loopback socket; the Pi permits only one processing owner. Version 7 of that
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

## Processing station and latency

Studio is the voice processing station. Its compute supports Qwen speech
recognition, the configured agent, and expressive Chatterbox synthesis. The Pi
owns capture, Rustpotter, endpointing, playback, and character animation; it
runs no speech-recognition or speech-synthesis model. Conversational voice
requires the paired Studio app open on an awake Mac.

The pipeline waits for an endpointed utterance before transcription, then uploads
Chatterbox chunks as they arrive while the Pi prebuffers for playback. Response
latency therefore includes endpointing, transport, ASR, agent response, TTS,
WAV upload, and playback startup. The worker reports ASR, agent, and synthesis
durations; these do not constitute a complete end-to-end latency measurement.
Measure the stages on the deployed setup before changing model quality or
introducing streaming.

## Direction evidence

Direction uses at most 30 accepted stereo frames from the last three seconds.
Silence and rejected frames add no votes; old votes expire even when no further
frames arrive. The `confidence` field measures vote agreement, not a calibrated
probability of identifying the speaker. At least five votes and 75% agreement
are required for a known side.

At wake confirmation, the listener checks the age of the oldest vote supporting
the selected side, rather than timestamping utterance completion as new evidence.
Evidence aged three seconds or more cannot trigger attention. Microphone spacing
and channel orientation must still be explicitly commissioned; their default
values disable direction-based attention.

## Streaming replies and timing

Chatterbox generates native audio chunks on Studio. The Studio worker uploads ordered PCM16 mono 24 kHz WAV chunks directly to
the authenticated gateway. UI observers receive status and timing events; they
have no upload or completion responsibility. `POST /api/v2/speech/stream` creates one
runtime speech run, `/api/v2/speech/{run}/chunks/{sequence}` appends audio, and
`/api/v2/speech/{run}/end` declares the final sequence. The existing complete-WAV
endpoint remains available for other callers.

The Pi prebuffers two seconds (or a shorter complete reply), then feeds one
`aplay` process continuously. Upload end does not mean playback completion.
Chunks are bounded to two seconds and the whole reply to 120 seconds. Out-of-order
chunks are rejected; cancellation, upload stalls and buffer exhaustion terminate
the run. Generation slower than real-time can exhaust the prebuffer; its physical
behavior still needs commissioning.

The runtime analyzes accumulated audio and extends the character spline under
its existing motion run ID. Extension starts from the commanded position and
velocity, keeps the immutable anchor and calibration checks, and retains head-led
staging, secondary body beats and clip variation. Network chunks do not become
separate gestures. Open-stream plans carry a short continuation horizon; terminal
playback triggers the existing final settle. Software player elapsed time drives
the audio frame estimate; ALSA buffering and physical motion require live review.

Voice → Debug reports capture after wake detection, transcription, agent time,
first synthesized chunk, total synthesis, summed upload round trips, and Pi
queue-to-player-start/elapsed durations. These are monotonic durations measured
within each process; overlapping stages cannot be added as total latency.
Player start is software timing, not an acoustic measurement at the speaker.
Capture duration requires the updated Pi listener; older listeners omit it.

## Local interaction feedback

The listener queues a semantic wake event to `oriond` before it sends anything
to the processing station. Endpoint completion queues thinking before utterance
upload. Feedback events carry the Pi voice session ID. Runtime guards suppress
duplicates and stale transitions, and enforce a bounded feedback lease.

Wake feedback uses one quiet tone and a brief three-colour pulse, followed by
steady listening light. Thinking uses one quiet entry cue, warm-white breathing
and small head motion through the existing calibrated compiler. Buffered
follow-up audio survives a bare-wake transition back to listening. Local
capture continues throughout cues; acoustic echo cancellation is not implemented.

Session-scoped speech uploads bind the Pi speech run to the active voice turn.
Thinking yields when the runtime starts its player, preserving commanded spline
position and velocity, the conversational anchor and speech variation. Cues
cannot displace speech. Stop/rest and foreground work retain their priority.
When Studio is unavailable after capture, the runtime plays `error_muted` once
and returns to listening. Physical cue loudness and microphone pickup remain
unverified.
