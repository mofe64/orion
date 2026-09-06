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
  -> speech animation -> terminal playback acknowledgement
  -> echo guard -> five-second conversation invitation -> command or wake rearm
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
The post-response window also reuses this frozen threshold.
These energy rules are a prototype based on Pi measurements; clipping,
continuous background speech, and quiet commands still require physical evaluation.

If the first transcript is only the wake phrase, Studio requests a follow-up
command. The Pi buffers up to one endpointed follow-up while Qwen is working,
so the transition does not discard speech spoken during confirmation. Empty
commands fail without invoking the agent.

Processing and playback suppress further wake triggers. Playback acknowledgement
is sent only after the Pi reports terminal playback. After successful playback,
a compatible worker requests a post-response window. Failed or cancelled
playback returns directly to wake listening.

The Pi discards the first 0.5 seconds after acknowledgement, then requires
300 ms of consecutive below-threshold audio before accepting another turn.
If no quiet baseline appears within two seconds, it abandons the window.
Once ready, Orion shows a silent, soft teal pulse on a 1.4-second cycle and accepts a command without
"Hey Orion" for five seconds. Speech onset requires 180 ms of sustained energy;
a short onset that begins before the deadline may finish confirmation just
after it. A 300 ms pre-roll preserves the beginning of the command. The guard
audio is never included. Once onset is confirmed, the ordinary endpoint and
120-second turn lease apply instead of the invitation deadline.

Each accepted follow-up gets a fresh voice session ID linked to the preceding
completed turn. Studio validates that link and transcribes it as a command,
without a second wake-phrase check. Timeout, mute, disconnect, or cancellation
clears the listening window. The listener advertises `conversationWindow` in
its ready response; workers request it explicitly on `session.finish`, so older
peers retain one-shot behavior. Deploy the Pi runtime/listener and restart the
updated Studio worker together to enable the pulse and follow-up behavior.

This is turn-taking, not acoustic echo cancellation or barge-in. Speak after
the teal invitation appears. Sustained noise or delayed echo can still cause a
false onset; threshold, guard, and pulse timing require physical acceptance. Session deadlines, bounded socket queues and
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

## Agent conversation and memory

Voice session IDs identify capture/playback turns, not agent conversations.
`CodexAgentProvider` creates one ephemeral Codex thread when models load and
reuses it for every confirmed command, including post-response follow-ups and
later wake-word requests. Pi transport reconnects reuse the loaded models.
Worker restart or model reload creates a new thread; there is no idle-time
rotation, explicit new-conversation command, or persisted thread-resume policy.
The five-second listening deadline does not erase agent context.

The configured base instructions are `ORION_INSTRUCTIONS` in
[`agent.py`](../../orion_studio/voice_worker/orion_voice_worker/agent.py). They
request a conversational desk-lamp reply of at most two concise spoken
sentences, prohibit tools/file operations, and prohibit claims of physical
actions. The wrapper caps spoken output at 800 characters.

Orion does not load a `soul.md`, user profile, durable memory store, or memory
retrieval/write pipeline. Conversation context lasts with the worker; it is not
durable personal memory. A future memory design should distinguish character
instructions from user facts and retain explicit provenance for saved facts.

The agent boundary currently exposes `respond(text) -> str` and `close()`.
Orion registers no robot tools or structured tool-result dispatcher. The Codex
runtime is launched with read-only sandboxing and denied approvals, but the
prompt's no-tools instruction is not itself an enforced tool allowlist. Adding
robot tools requires explicit schemas, authorization, validated gateway calls,
timeouts/cancellation, and results fed back to the agent. Physical operations
must remain owned by `oriond`.

## Processing station and latency

Studio is the voice processing station. Its compute supports Qwen speech
recognition, the configured agent, and expressive Chatterbox synthesis. The Pi
owns capture, Rustpotter, endpointing, playback, and character animation; it
runs no speech-recognition or speech-synthesis model. Conversational voice
requires the paired Studio app open on an awake Mac.

The pipeline waits for an endpointed utterance before transcription, then buffers
Chatterbox audio locally before starting ordered uploads to the Pi. Response
latency therefore includes endpointing, transport, ASR, agent response, TTS,
WAV upload, and playback startup. The worker reports ASR, agent, and synthesis
durations; these do not constitute a complete end-to-end latency measurement.
Measure the stages on the deployed setup before changing model quality or
buffering thresholds.

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

Chatterbox generates native audio chunks on Studio from sentence segments bounded
to 160 characters, splitting long sentences at word boundaries. Each segment resets
the decoder context; gain remains fixed across the reply. A producer generates
audio independently of uploads through an eight-chunk queue. Cancellation waits
for any in-flight native inference before closing its generator or reusing the model.
The Studio worker uploads ordered PCM16 mono 24 kHz WAV chunks directly to
the authenticated gateway. UI observers receive status and timing events; they
have no upload or completion responsibility. `POST /api/v2/speech/stream` creates one
runtime speech run, `/api/v2/speech/{run}/chunks/{sequence}` appends audio, and
`/api/v2/speech/{run}/end` declares the final sequence. The existing complete-WAV
endpoint remains available for other callers.

Before the first upload, Studio collects at least six seconds of audio. Its startup
reserve is the larger of six seconds or twice the longest measured generation step
plus two seconds. After collecting six seconds, generation taking more than 75% of
the audio duration, or a required reserve above twelve seconds, selects complete-reply
buffering. Short replies also finish generation before upload. The 120-second audio
limit bounds this local buffer to about 5.8 MB of PCM; it does not extend the existing
Pi voice-session deadline. Slower generation therefore increases the wait before speech.

The Pi still prebuffers two seconds (or a shorter complete reply), then feeds one
`aplay` process continuously. Upload end does not mean playback completion.
Chunks are bounded to two seconds and the whole reply to 120 seconds. Out-of-order
chunks are rejected; cancellation, upload stalls and buffer exhaustion terminate
the run. The initial measurements cannot guarantee future generation or network
speed: a later slowdown can still exhaust the buffer. Sentence transitions and
long-reply playback with this buffering policy still require physical acceptance.

Worker stderr records `speech.buffer_ready` and `speech.chunk` events with request
IDs, the buffering decision, audio duration, generation time and upload time. The
runtime journal records `speech.chunk_received` with run IDs, sequence numbers and
remaining audio estimates, plus `speech.failed` with separate `buffer_exhausted`
and `upload_timeout` reasons. Speech status exposes `buffered_ms`: received duration
minus software playback elapsed, which can become negative. It is not a hardware
buffer measurement. Diagnostics contain no reply text, PCM or credentials.

The runtime analyzes accumulated audio and extends the character spline under
its existing motion run ID. Extension starts from the commanded position and
velocity, keeps the immutable anchor and calibration checks, and retains head-led
staging, secondary body beats and clip variation. Network chunks do not become
separate gestures. Extensions wait until the current gesture and any quiet hold
finish, and only reached gesture checkpoints advance variation history.
Open-stream plans carry a short continuation horizon. The end marker revises
that plan at a gesture boundary without requiring another chunk, and late finalization
installs only a settle. Terminal playback blends an executing performance into
a settle from commanded position and velocity; an existing settle continues.
Software player elapsed time drives
the audio frame estimate; ALSA buffering and physical motion require live review.

Runtime logs include `speech.stream_end` and `speech.terminal` with the speech
run ID. `speech.motion_finalized` and `speech.motion_settle` identify the motion
run, remaining audio estimate, and commanded or measured-fallback handover.
These events distinguish normal finalization from buffer failure and recovery;
they do not measure acoustic timing or physical joint smoothness.

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
steady listening light. Thinking uses one quiet entry cue and a three-second
breath through the same amber, teal and lavender palette. Its visible diagonal
head tilt leads delayed shoulder/elbow support, with opposing preparation and
an asymmetric counter-tilt. The existing thinking style and calibrated compiler
preserve the conversational anchor and smooth speech handover. Buffered
follow-up audio survives a bare-wake transition back to listening. Local
capture continues throughout cues; acoustic echo cancellation is not implemented.

Session-scoped speech uploads bind the Pi speech run to the active voice turn.
Thinking yields when the runtime starts its player, preserving commanded spline
position and velocity, the conversational anchor and speech variation. Cues
cannot displace speech. Stop/rest and foreground work retain their priority.
When Studio is unavailable after capture, the runtime plays `error_muted` once
and returns to listening. Physical cue loudness and microphone pickup remain
unverified.
