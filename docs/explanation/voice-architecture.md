# Orion voice architecture

The Raspberry Pi owns Orion's microphone and Rustpotter wake detector. Studio
hosts a reusable Rust coordinator and agent runtime. The coordinator owns a
Python Qwen3-ASR/Chatterbox inference worker. Studio never opens
its workstation microphone or loads a wake detector.

## Audio and control flow

```text
Pi ReSpeaker stereo capture (16 kHz signed PCM16)
  -> local coarse direction observation before downmixing
  -> mono Rustpotter + three-second in-memory pre-roll
  -> wake candidate notification over token-authenticated WebSocket
  -> Pi speech endpoint -> bounded complete utterance upload
  -> Rust coordinator sends an ASR job to the Python speech worker
  -> Rust confirms "Hey Orion" and extracts the command
  -> in-process AgentHandle -> Rust agent runtime -> Codex App Server
  -> Rust sends a synthesis job to the Python speech worker
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
A disconnected session is discarded; the Rust coordinator reconnects automatically.
Processing has a 120-second session lease; entering playback grants 180 seconds.

## Transport and deployment

Studio saves the paired gateway address and token in the OS credential store.
Gateway reconnect restores status/authoring connectivity without replaying robot
operations. Paired Studio starts its coordinator while the app is open. The listener token is reused from
that saved connection. See [pairing configuration](../reference/configuration.md#saved-pairing).

The Pi listener uses plain WebSockets on port 7448 and the Pi's existing
Studio token for authentication. Studio derives `ws://GATEWAY_HOST:7448/`
from the gateway connection; `ORION_PI_VOICE_URL` can override it with another
`ws://` endpoint. No certificate setup is required. Tokens and audio travel
unencrypted, so this development connection is intended for a trusted LAN.
Token authentication controls access but does not protect against network
interception.

Studio starts the top-level `orion-coordinator` library and stops it when the
app exits. Closing the Voice panel only detaches its status connection. The
coordinator owns Pi credentials and connects directly to the Pi; the Python
speech worker receives only model configuration and inference jobs through its
private stdin/stdout pipes. It has no Pi connection or agent client.

The coordinator exposes status through an authenticated loopback WebSocket;
the Pi permits only one processing owner. Version 7 of the observer protocol
rejects workstation microphone frames and UI playback claims. Pi protocol
version 1 uses JSON session messages and length-checked PCM16 utterances.
The existing Studio command name `start_voice_worker` starts this coordinator.

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
The top-level [`orion-agent` crate](../../agent/README.md) owns one ephemeral
Codex thread and reuses it for confirmed commands, post-response follow-ups,
and later wake-word requests. Studio compiles this library through a Cargo path
dependency and owns its service separately from the coordinator and speech worker.

Pi reconnects and idle coordinator/model reloads preserve the agent thread.
Quitting Studio, changing the agent model/effort or executable, and failed or
cancelled active agent requests retire it. The next request starts a fresh
thread. There is no idle-time rotation, explicit new-conversation command, or
persisted thread-resume policy. The five-second listening deadline does not
erase agent context.

The coordinator calls `AgentHandle::respond` directly through bounded Rust
message channels. Each request has a separate reply channel; dropping an active
call cancels its Codex turn and retires that uncertain conversation. There is no
agent TCP server or Python agent client. The independent agent executor survives
coordinator restarts while Studio retains `AgentService`.

The coordinator owns wake confirmation, follow-up state, voice request IDs,
Pi reconnection, buffering, uploads, and playback acknowledgement. Python owns
only ASR and TTS model execution. A speech job has an ID; synthesis responses
have ordered chunk sequence numbers and an explicit end marker. Channel closure
without that marker is failure, never permission to upload held startup audio.

The base instructions live in
[`agent/src/prompt/mod.rs`](../../agent/src/prompt/mod.rs). Orion uses Codex's
built-in live web search and three client-executed tools: `append_memory`,
`search_memories`, and `set_lighting`. Dynamic tool calls are bound to the active
thread and turn, limited to 16 per turn, and validated before execution.
Unexpected interactive requests are rejected. Desktop shell, browser, app,
plugin, and multi-agent features are explicitly disabled for Orion's thread.
The installed App Server dynamic-tool API is experimental.

Memory entries are stored in a local `MEMORY.md` with IDs, UTC creation dates,
and reserved delimiters. Writes use a file lock and atomic replacement; repeated
identical entries reuse the existing entry. Retrieval uses bounded keyword
matching, returning at most eight entries. The agent saves only explicitly
requested memories and treats retrieved text as data. Current UTC time is
supplied on each turn. See [agent storage and tools](../../agent/README.md).
Studio Settings exposes curated personality choices and memory management.
Personality selections generate local `SOUL.md` instructions; free-form prompt
editing is not exposed. Profile changes wait behind any active agent request,
then retire its conversation without restarting speech models. Revision checks
reject stale personality saves and conflicting memory edits. Profile reads
preserve conversation context. Automatic memory collection is not implemented.

`AgentHandle::respond_with_events` emits request-scoped activity while awaiting
a final answer. Search emits one acknowledgement per agent turn. The coordinator
synthesizes and plays it while Codex continues; after playback, `session.processing`
restores the Pi's thinking reaction and breathing light. Capture remains
suppressed. Final speech waits for intermediate playback to finish, and only
final playback completion opens the follow-up window. Memory tools are silent.
Older listeners without `toolFeedback` continue processing without spoken
search acknowledgements.

Lighting tool choices resolve into validated brightness, effect, and RGBW
palette parameters. The coordinator sends them through the authenticated
`lamp_effect` gateway operation. `oriond` stores the lamp program beneath speech,
scene, and voice-feedback lighting; it reappears after those finish. Animated
effects without a palette use warm white plus one randomly selected accent.
Brightness-only changes preserve the current effect and palette. A paired Pi
with the updated gateway and runtime is required; errors are returned to the
agent rather than reported as successful actions.

Final spoken output is bounded to 800 Unicode characters plus an ellipsis;
Codex citation markers are removed from speech. Only matching final assistant
messages become the answer. The deterministic search acknowledgement is separate
from model commentary. Physical operations remain owned by `oriond`.

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
audio independently of uploads through an eight-chunk Rust queue. Cancellation
of an active native inference job terminates that Python process; the next job
loads fresh models. Completed jobs keep the worker and models available. Speech
jobs have a 240-second host deadline, but the Pi session lease still bounds a
voice turn.
The Rust coordinator uploads ordered PCM16 mono 24 kHz WAV chunks directly to
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

Coordinator stderr records `speech.buffer_ready` and `speech.chunk` events with request
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
