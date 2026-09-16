# Orion voice architecture

Orion’s Pi runs the complete speech pipeline and the Rust agent coordinator.
Rustpotter detects wake candidates, Silero finds speech boundaries, Qwen3-ASR
transcribes, and Pocket TTS generates replies. Codex App Server runs on the Pi
with a subscription login; model inference and web search use online services.
Studio is an optional settings, authoring, and observation client.

## Audio and control flow

```text
Pi ReSpeaker stereo capture (16 kHz signed PCM16)
  -> local coarse direction observation before downmixing
  -> mono Rustpotter + three-second in-memory pre-roll
  -> wake candidate notification over token-authenticated WebSocket
  -> short wake-prefix ASR alongside continued command capture
  -> confirmed wake starts homing while the microphone keeps recording
  -> Silero endpoint -> bounded complete utterance over loopback
  -> Rust coordinator sends an ASR job to the Python speech worker
  -> Rust confirms "Hey Orion" and extracts the command
  -> in-process AgentHandle -> Rust agent runtime -> Codex App Server
  -> explicitly final answer sentences -> Pocket worker
  -> ordered WAV chunk uploads -> one oriond-owned streaming player
  -> speech animation -> terminal playback acknowledgement
  -> echo guard -> five-second conversation invitation -> command or wake rearm
```

Rustpotter is the only active wake detector. Its reference and native adapter
live under `voice/`; Studio does not depend on that package. Qwen retains the
second-stage false-positive check. Rejected wake candidates never reach the
agent or trigger directional attention. Qwen confirmation is not a guarantee
that every false positive is eliminated.

After confirmation, the Pi can prepare the body while the coordinator processes the
command. The runtime owns [automatic rest and waking](system-architecture.md#automatic-rest-and-waking),
including the activity deadline and torque checks. Microphone mute remains
independent of that lifecycle.

## Capture ownership and session lifecycle

The Pi listener is an independent process, outside the 50 Hz motor loop. The
service opens capture when it starts unless a saved mute preference disables it.
Closing Studio or disconnecting its processing station leaves capture running.
One authenticated connection owns processing; additional processing connections
are refused. Separate authenticated control connections can inspect or change
mute. Muting closes capture, clears buffered audio and persists across restarts.

Every interaction has a random session ID. The Pi captures continuously while
enabled, keeps three seconds of pre-roll in memory and sends a candidate event
immediately. Peers negotiate `wakePrefix: true` in the processing handshake.
The listener then sends up to two seconds before detection plus 200 ms after
detection as a `wake_prefix` ASR job, while continuing the same recording.
Prefix text can confirm the wake but never supplies an agent command. An
inconclusive prefix falls back to confirmation using the complete utterance.
Older peers retain complete-utterance confirmation.

The full recording includes every command sample, including speech overlapping
prefix verification. If endpointing wins the race, the listener holds the
complete utterance until the prefix result arrives, and preserves any follow-up
speech during both ASR passes. Only one ASR job owns a session at a time. Audio
stays bounded and is cleared on mute, cancellation or disconnect.
The capture limit is 30 seconds after wake detection and
33 seconds including pre-roll. Hitting the limit produces `max_duration`;
the coordinator rejects that audio without sending an incomplete command to
the agent.

The managed Pi listener uses Silero ONNX with recurrent state per stream.
Twenty-millisecond capture frames are accumulated into 512-sample Silero
windows with 64 samples of context; no silence is inserted between frames.
Unmute acknowledgement and processing readiness wait for microphone startup.
Speech probability uses 0.5 onset and 0.35 release thresholds. A fixed 2× gain
applies only to Silero’s input, with clipping protection; Rustpotter and Qwen
receive unchanged PCM. Capture ends after 1.2 seconds without speech, subject
to a 1.2-second minimum capture and the 30-second limit. This gain and hold time
addressed early endings in the retained quiet ReSpeaker recordings.

The same speech classifier handles the post-response quiet guard and follow-up
onset, with fresh recurrent state at each boundary. Legacy development sessions
without `--vad-model` retain the energy detector. The installed service always
supplies the pinned Silero model.

If the first transcript is only the wake phrase, the coordinator requests a follow-up
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
"Hey Orion" for five seconds. Speech onset requires 180 ms of sustained speech;
a short onset that begins before the deadline may finish confirmation just
after it. A 300 ms pre-roll preserves the beginning of the command. The guard
audio is never included. Once onset is confirmed, the ordinary endpoint and
120-second turn lease apply instead of the invitation deadline.

Each accepted follow-up gets a fresh voice session ID linked to the preceding
completed turn. The coordinator validates that link and transcribes it as a command,
without a second wake-phrase check. Timeout, mute, disconnect, or cancellation
clears the listening window. The listener advertises `conversationWindow` in
its ready response; workers request it explicitly on `session.finish`, so older
peers retain one-shot behavior. Deploy the Pi runtime/listener and restart the
updated coordinator together to enable the pulse and follow-up behavior.

Speak after the teal invitation appears. Acoustic echo cancellation and
interruption during playback are not implemented. Sustained noise or delayed
echo can cause a false onset; threshold, guard, and pulse timing require
physical acceptance. Session deadlines, bounded socket queues, and state checks
limit buffering and prevent stale command replay.
A disconnected session is discarded; the Rust coordinator reconnects automatically.
Processing has a 120-second session lease; entering playback grants 180 seconds.

## Confirmed waking

When the coordinator accepts `wake.verified` or sends `wake.confirmed`, the Pi listener forwards
`voice SESSION confirmed` to `oriond`. It uses the same ordered queue as wake
candidate, prefix-verification and endpoint events, then waits up to five seconds for the runtime
acknowledgement. Confirmation is required even when microphone direction is
unknown. A rejected or undelivered confirmation fails the voice connection.
The runtime accepts the current session's confirmation once, so duplicate
delivery cannot keep extending the inactivity deadline.

While Orion rests, the listener still detects and captures wake candidates.
Unconfirmed candidates play the wake chime while leaving the body still and the
light off. The runtime requires `voice SESSION verify` or an
endpoint before it accepts confirmation. A confirmed wake
starts the return home. If confirmation arrives during descent, the runtime
finishes that movement before returning home. Explicit Character Stop and
maintenance mode require an explicit character start to enable automatic waking.

Capture continues during homing. The listener retains a buffered follow-up
command spoken while ASR confirms a bare wake phrase. The coordinator can also process a
command included in the original utterance while the body moves. Once home
finishes, the runtime applies the latest listening or thinking state and checks
whether optional direction evidence is still fresh enough for attention.

A reply uploaded for a voice session stays queued until the runtime recognizes
it as part of an active confirmed conversation and home and any accepted
attention movement have finished. Upload
timeouts continue to apply while playback waits. Cancellation or session expiry
removes the queued reply; a rest lifecycle fault cancels it and requires explicit
recovery. These checks prevent a delayed reply from playing after its turn has
ended.

## Transport and deployment

Studio saves the paired gateway address and token in the desktop credential
store. The onboard coordinator uses the Pi’s token file and connects to the
listener and gateway through loopback. `--local-processor` reserves the listener’s
processing connection for a local process; authenticated remote control clients
can still read microphone status or mute it.

Studio discovers onboard ownership at `/api/v2/voice/status`. Settings, profile,
and microphone commands pass through the gateway’s allowlisted
`/api/v2/voice/request` route. `/api/v2/voice/events` returns at most 33 events
with a coordinator generation and increasing event IDs. Studio polls these
snapshots, discards duplicates, and resets its cursor after a coordinator restart.
It does not receive PCM or acknowledge playback. The gateway never exposes the
private service or observer tokens.

Private stdin/stdout pipes carry model jobs. The existing HTTP gateway and
`oriond` streaming-player contract carry response WAV chunks and hardware control.
Remote Studio traffic uses the existing token-authenticated trusted-LAN HTTP
connection; the onboard audio path does not cross that LAN connection.

Follow [Pi installation](quickstart.md#pi-local-voice-and-agent). The legacy
Apple Silicon processing adapter remains available for development.

## Orion service lifecycle

The [orion-service](../orion-service/README.md) crate hosts the Pi agent and voice
coordinator. Systemd starts `orion-voice-stack` at boot with `ORION_ONBOARD=1`.
Studio's desktop backend only stores pairing and forwards requests to the paired
Pi gateway. It cannot create an embedded owner or attach to a Mac background host.
A missing or unavailable onboard service returns an error.

The Pi service publishes a random loopback address and token in a private service
directory. The gateway forwards bounded, authenticated JSON requests to it.
Repeated starts with matching configuration reuse the coordinator. The service
control protocol and coordinator observer protocol have separate credentials.

An OS file lock prevents simultaneous Pi owners and is released on process exit.
Settings changes are serialized with coordinator startup; profile edits use the
agent's existing request queue. At startup the service reads the Pi token file
and saved settings, then retries a stopped coordinator every five seconds.
SIGTERM stops the coordinator, cancels its speech run, and shuts down the agent
and speech workers. Restarting starts a fresh conversation while preserving saved
memory and personality. See the [quickstart](quickstart.md#pi-local-voice-and-agent).

## Agent and physical boundary

Raw microphone audio stays on the Pi, where Qwen and Pocket run. The Codex provider receives confirmed command text. The agent
produces spoken replies and can request lamp changes through `set_lighting`.
The tool validates brightness, effect, and color choices and sends them through
the coordinator and authenticated gateway to `oriond`. Motion control is outside
the agent's available tools.

The Pi may request allowlisted character reactions based on session events.
Confirmed, confident direction observations can request the runtime's approved
attention turn. A resting character first returns home. A character explicitly
stopped by the user remains off until an explicit start; voice capture can stay
enabled in either state.

## Agent conversation and memory

Voice session IDs identify capture/playback turns, not agent conversations.
The top-level `orion-agent` [crate](../agent/README.md) owns one ephemeral
Codex thread and reuses it for confirmed commands, post-response follow-ups,
and later wake-word requests. The headless host compiles this library through a Cargo path dependency and
owns its service separately from the coordinator and speech workers.

Pi reconnects and idle coordinator/model reloads preserve the agent thread.
Stopping the headless service, changing the agent model/effort or executable, and failed or
cancelled active agent requests retire it. The next request starts a fresh
thread. There is no idle-time rotation, explicit new-conversation command, or
persisted thread-resume policy. The five-second listening deadline does not
erase agent context.

The coordinator calls `AgentHandle::respond_with_events` through bounded Rust
message channels. Each request has a separate reply channel; dropping an active
call cancels its Codex turn and retires that uncertain conversation. There is no
agent TCP server or Python agent client. The independent agent executor survives
coordinator restarts while the host retains `AgentService`.

The coordinator owns wake confirmation, follow-up state, voice request IDs,
Pi reconnection, buffering, uploads, and playback acknowledgement. Python owns
only ASR and TTS model execution. A speech job has an ID; synthesis responses
have ordered chunk sequence numbers and an explicit end marker. Channel closure
without that marker is failure, never permission to upload held startup audio.

The base instructions live in
`[agent/src/prompt/mod.rs](../agent/src/prompt/mod.rs)`. Orion uses Codex's
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
supplied on each turn. See [agent storage and tools](../agent/README.md).
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
scene, and voice-feedback lighting; it can reappear after those finish when
the rest lifecycle permits light output. Rest darkness suppresses the output
while preserving the preference. Animated effects without a palette use warm
white plus one randomly selected accent.
Brightness-only changes preserve the current effect and palette. A paired Pi
with the updated gateway and runtime is required; errors are returned to the
agent rather than reported as successful actions.

Final spoken output is bounded to 800 Unicode characters plus an ellipsis;
Codex citation markers are removed from speech. Only matching final assistant
messages become the answer. The deterministic search acknowledgement is separate
from model commentary. Physical operations remain owned by `oriond`.

## Processing station and latency

Speech runs on the Pi. Keep the ASR and TTS workers resident; the managed
profile assigns three inference threads to each and runs Silero on one thread.
Wake processing is suppressed while an answer is being prepared or played.

End-to-end response time includes trailing silence, transcription, Codex network
and model latency, synthesis buffering, and actual playback startup. A fast
first TTS chunk does not guarantee a fast audible reply. Pocket FP32 can generate
more slowly than playback; it therefore needs complete-reply buffering on this
Pi. INT8 reduces synthesis time but changes the sound. Settings exposes both,
with FP32 and Alba as the initial quality preference.

The physical test harness measures from source playback completion to observed
reply playback and records the stage timings separately. See the single speech
evaluation report for measured results and limitations.

## Direction evidence

Direction uses at most 30 accepted stereo frames from the last three seconds.
Silence and rejected frames add no votes; old votes expire even when no further
frames arrive. The `confidence` field measures vote agreement, not a calibrated
probability of identifying the speaker. At least five votes and 75% agreement
are required for a known side.

The listener measures age from the oldest vote supporting the selected side.
After the runtime acknowledges confirmation, the listener sends any accepted
side with its observation age. Time spent waiting in the local command queue
also counts toward that age.

The runtime checks freshness again when it can begin attention after home.
Evidence must still be younger than three seconds. Homing therefore consumes
the same freshness budget as ASR and command delivery. Stale evidence or a
refused attention turn leaves Orion facing home and allows the voice turn to
continue. Microphone spacing and channel orientation require physical
calibration; their default values disable directional attention.

## Streaming replies and timing

The Codex adapter accepts streamed text only for a matching thread, turn, and
item explicitly marked `final_answer`. Complete sentences feed TTS while Codex
finishes. Unknown phases wait for final completion. Citation markup and model
commentary never enter speech. The authoritative final text must preserve any
already-emitted prefix; a mismatch cancels the turn.

The coordinator renders successive sentences through one TTS worker and one
runtime speech run. The selected voice is captured when the response starts,
so a preset change applies to the next response. Queues are bounded to eight
items. Cancelling native inference retires only the affected ASR or TTS worker;
completed jobs keep both models resident. Speech jobs have a 240-second host
deadline, but the listener’s session lease also bounds the voice turn.

The Rust coordinator uploads ordered PCM16 mono 24 kHz WAV chunks directly to
the authenticated gateway. UI observers receive status and timing events; they
have no upload or completion responsibility. `POST /api/v2/speech/stream` creates one
runtime speech run, `/api/v2/speech/{run}/chunks/{sequence}` appends audio, and
`/api/v2/speech/{run}/end` declares the final sequence. The existing complete-WAV
endpoint remains available for other callers.

Before the first upload, the coordinator collects at least six seconds of audio. Its startup
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

When the runtime permits feedback, a wake candidate produces one quiet tone and
a brief three-colour pulse, followed by steady listening light. Thinking uses
one quiet entry cue and a three-second
breath through the same amber, teal and lavender palette. Its visible diagonal
head tilt leads delayed shoulder/elbow support, with opposing preparation and
an asymmetric counter-tilt. The existing thinking style and calibrated compiler
preserve the conversational anchor and smooth speech handover.
Repeated thinking notifications preserve the running gesture and its timing;
moving from transcription to agent processing does not restart the opening tilt.
Buffered follow-up audio survives a bare-wake transition back to listening. Local
capture continues throughout cues; acoustic echo cancellation is not implemented.

Session-scoped speech uploads bind the Pi speech run to the active voice turn.
Thinking yields when the runtime starts its player, preserving commanded spline
position and velocity, the conversational anchor and speech variation. Cues
cannot displace speech. Stop/rest and foreground work retain their priority.
When Studio is unavailable after capture and the runtime permits feedback, it
plays `error_muted` once and the listener returns to wake detection. During
descent, waking, or a rest lifecycle fault, the runtime suppresses voice
cues and immediate reaction changes. At mechanical rest, only the candidate's
wake chime is allowed; body reactions still wait for confirmation and homing.
Waking permits light output and restores
the latest voice reaction after home completes. Speaker-to-microphone tests
occupy the same playback device as the wake chime, so chime playback is checked
separately while the body remains at rest.
