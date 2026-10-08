# Orion voice architecture

Orion's Pi runs microphone capture, wake detection, speech recognition, speech
synthesis and the agent coordinator. Rustpotter detects a possible wake phrase,
an openWakeWord phrase verifier confirms it acoustically, Silero finds the end of speech, Qwen3-ASR transcribes, and Piper Alba Medium
produces the reply. Codex App Server runs on the Pi
and uses online model inference. Studio provides settings and observation through
the gateway.

## Audio and control flow

```text
ReSpeaker stereo capture
  -> direction observation and mono downmix
  -> Rustpotter wake candidate -> acoustic phrase verification
  -> accepted: chime, three-color light pulse and wake confirmation
  -> Silero endpoint -> Qwen transcription and wake-phrase check of the complete recording
  -> confirmed command -> Rust agent -> Codex App Server
  -> final answer sentences -> Piper Alba TTS -> coordinator audio buffer
  -> local gateway -> oriond playback and speech animation
  -> playback completion -> echo guard -> follow-up listening window
```

The [system architecture](system-architecture.md#system-boundary) shows the
processes and connections. The listener owns capture and voice-session state.
The coordinator owns inference jobs, command validation, agent calls, response
buffering and playback acknowledgement. Python workers execute ASR or TTS jobs.
`oriond` owns physical playback, animation and the rest/wake sequence.

## Capture ownership and session lifecycle

The listener opens capture on startup unless the saved microphone preference
is muted. It runs independently of the motor loop and keeps recording during
mechanical rest. Muting closes capture, clears buffered audio and saves the
preference. One authenticated loopback WebSocket owns processing; separate
control connections can read or change mute.

Capture arrives as 20 ms stereo frames at 16 kHz. The listener estimates direction
before downmixing to mono and retains three seconds of audio in memory. On V2,
processed channel 0 supplies mono capture; optional calibrated XVF3800 direction
reads run in a separate 20 Hz daemon thread. Only beam-3 samples above the energy
threshold vote. At least five votes in three seconds and 75% agreement are needed;
the first supporting vote supplies the evidence age. Missing calibration or a
USB failure suppresses attention without affecting capture. See
[V2 calibration](../voice/README.md#calibrate-v2-attention). When
Rustpotter detects a candidate, the listener assigns a random session ID and
registers a silent runtime session before notifying the coordinator.

### Acoustic wake verification

The packaged verifier in `voice/models/verifier/` scores every captured frame
with openWakeWord's frozen melspectrogram and embedding models and a small
“Hey Orion” classifier. It streams 80 ms chunks and ignores its first 26
chunks after capture opens. [The packaged configuration](../voice/models/verifier/config.json)
holds the decision rule. A candidate is accepted when a score reaches the
configured threshold between 0.8 seconds before and 1 second
after the Rustpotter candidate; otherwise the verifier rejects it at the
1-second deadline. A score from before the candidate accepts at the candidate,
so a typical acceptance adds no delay.

Acceptance sends `wake.verified` with `source: "acoustic"` to the coordinator
and replaces the Qwen prefix pass. Rejection does not end the session: capture
continues and Qwen checks the complete recording, as it does for an
inconclusive prefix. Qwen's complete-recording wake check remains the final
authority in both cases. An endpoint that arrives before the verdict is held
until the verdict is sent.

The verifier falls back to the prefix pass below when more than a tenth of
its recent chunks take over 40 ms, or when the listener starts with
`--no-verifier`. The ready message reports the verifier's settings and
whether it is active.

### ASR prefix fallback

With the negotiated `wakePrefix` capability and no active verifier, the
listener sends up to two seconds before detection plus 200 ms afterward for
Qwen verification. Capture continues
into the same complete recording while that job runs. Prefix text can confirm
the wake phrase; the agent command always comes from a complete utterance. An
inconclusive prefix falls back to confirmation using the full recording.

If Silero detects the end before prefix verification finishes, the listener holds
the complete utterance until the prefix result arrives. One ASR job owns the
session at a time. If the complete transcript contains only the wake phrase, the
coordinator requests a follow-up command. The listener can retain one completed
follow-up spoken during verification, which preserves speech across that
transition.

The capture limit is 30 seconds after wake detection, or 33 seconds including
pre-roll. A recording that reaches the limit carries `max_duration`. The
coordinator rejects it and reports an error asking for a shorter request. Empty
commands and rejected wakes also stop before the agent call. Mute, cancellation,
disconnect and session expiry discard the audio.

### Speech boundaries

The installed listener uses Silero ONNX with recurrent state for each audio
stream. It accumulates capture frames into 512-sample windows with 64 samples of
context. This preserves continuous audio across frames. The classifier applies
2× gain only to its own input; the recorded PCM supplied to Rustpotter and Qwen
keeps its original level.

Silero enters speech at probability 0.5 and remains in speech down to 0.35. This
difference reduces repeated switching near the threshold. Capture ends after
1.2 seconds without speech and at least 1.2 seconds of recording. The same
classifier handles follow-up onset and the quiet guard, with fresh recurrent
state at each boundary. Microphone startup must finish before the listener
acknowledges unmute or reports processing readiness.

The standalone listener can use an energy detector when no VAD model is supplied.
The managed installation supplies Silero. Capture gain and wake sensitivity are
separate settings described in [configuration](configuration.md#pi-runtime-and-listener).

## Confirmed waking

With the acoustic verifier active, the wake chime and three-color light pulse
play only after the verifier accepts. The listener then confirms the wake
itself, which starts the return home at mechanical rest. A candidate the
verifier rejects stays silent; if Qwen later confirms the complete recording,
the chime and pulse play at that confirmation. Without the verifier, the
candidate plays the chime and pulse and Qwen confirmation starts the return
home. A rejected wake ends any light feedback without waking the body or
dispatching a command. Other lighting remains suppressed at rest.

The coordinator sends `wake.verified` after a successful ASR prefix or
`wake.confirmed` after confirming the complete utterance. The listener forwards
`voice SESSION confirmed` through its ordered runtime queue and waits up to five
seconds for acknowledgement. The runtime accepts one confirmation for the current
session, after verification or endpointing has begun. Duplicate delivery cannot
extend the inactivity deadline. An undelivered or rejected confirmation ends the
connection.

A confirmation received during descent lets that movement finish before home
begins. Capture continues throughout. After home completes, the runtime applies
the latest listening or thinking state and checks any direction evidence before
an attention turn. Explicit Character Stop and maintenance mode require an
explicit character start to rearm automatic waking.

A reply uploaded for a voice session remains queued until the runtime recognizes
an active confirmed conversation and finishes home and any accepted attention
movement. Cancellation, expiry or a rest fault removes the queued reply. Upload
timeouts still apply while it waits. See [automatic rest and waking](system-architecture.md#automatic-rest-and-waking).

## Follow-up conversation

Processing suppresses additional wake triggers; V2 playback accepts only
acoustically verified wake-phrase barge-in. After successful runtime playback,
the coordinator acknowledges completion to the
listener and requests a follow-up window. Failed or cancelled playback returns
to wake detection.

The listener discards the first 0.5 seconds after acknowledgement, then requires
300 ms of consecutive quiet audio. If quiet is not established within two
seconds, it closes the window. Once ready, Orion displays a soft teal pulse and
accepts a command without “Hey Orion” for five seconds.

Speech onset requires 180 ms of sustained speech. A 300 ms pre-roll preserves the
beginning of the command, excluding guard audio. Once onset is confirmed, normal
endpointing applies. The follow-up receives a fresh session ID linked to the
preceding completed turn. The coordinator validates that link and transcribes the
recording directly as a command.

Processing starts with a 120-second session lease; entering playback grants 180
seconds. During synthesis and playback, the coordinator sends `session.keepalive`
every five seconds. The listener extends its deadline and forwards `voice SESSION
keepalive`; the runtime extends its processing/thinking/speaking deadline without
changing phase, reaction, cue or history. Stale or expired sessions cannot renew.
`session.processing` and `session.playing` mark actual phase transitions. The lease expires if the owner disappears.
Disconnect, mute, cancellation and timeout clear the current session. The
coordinator reconnects automatically. Protocol capabilities allow older peers to
confirm the complete utterance or receive a single response without a follow-up
window.

V2 hardware echo cancellation supports wake-phrase barge-in during playback. V1
requires speaking after the teal invitation appears. Sustained noise or delayed
echo can still trigger an unwanted follow-up, so microphone and speaker behavior require
physical checks.

## Wake-phrase barge-in

On V2, Rustpotter runs during `playing` while a healthy acoustic verifier is
loaded. The listener keeps three seconds of pre-roll and verifies each candidate
without stopping playback. Rejection emits nothing. Acceptance emits
`session.interrupted` with `reason: "barge_in"` for the old session, then
`wake.candidate` and accepted `wake.verified` for a fresh ID. Capture starts from
the pre-roll, including speech during verification, and uses normal endpointing
and full-utterance Qwen validation. The listener sends `voice OLD cancel` before
new-session feedback and suppresses attention throughout that interruption turn.
Playback never uses the ASR-prefix fallback to authorize an interruption. Without
a healthy verifier, and on V1, playback wake detection stays off. Alarm dismissal
runs first and consumes its wake phrase without starting a command.

The coordinator reads the interruption reason. Barge-in during an active session
outside the responding phase is logged and ignored, preserving the session and
Pi connection. Accepted barge-in signals a per-response
stop token: final TTS, uploads, playback polling and search acknowledgement speech
stop, and the gateway cancels the active speech run. Remaining speech text is
consumed without synthesis. The response job keeps its agent reply channel open
until the turn finishes, outside the active voice session. Its completion cannot
finish or cancel the replacement session. Later physical tool requests from the
interrupted turn receive an interruption error. Alarm interruption still aborts
inference jobs and cancels playback.

`AgentService` dequeues requests one at a time in its dedicated runtime. A new
voice command can be captured and transcribed immediately, but its agent request
waits for the interrupted turn's terminal result. Keeping that result receiver
open preserves the ephemeral Codex conversation; dropping it would cancel the
turn and retire the conversation. The next command carries “The previous reply
was interrupted before it finished.” once, using the agent's pending delivery
notice. Direct sleep detection still uses the user's original command.

Runtime status and uploaded PCM do not identify the exact spoken text: buffered
audio may not have played. The notice therefore makes no claim about which words
were heard. The agent still retains its complete previous reply. Turn timeout,
protocol failure, mute, disconnect and alarms retain their existing cancellation
rules; a failed interrupted turn can still reset the conversation. See
[physical acceptance checks](../voice/README.md#check-v2-barge-in-on-the-lamp).

## Transport and deployment

The onboard coordinator reads the Pi token file and connects to the listener and
gateway through loopback. `--local-processor` restricts processing ownership to a
local connection. Private stdin/stdout pipes carry speech jobs and the Codex App
Server protocol; bounded Rust channels connect the coordinator to the agent.

Release activation keeps the listener, voice host and Studio gateway stopped
during the physical smoke test so they cannot dispatch competing speech or
movement. After the test, `character rest` establishes the runtime's `resting`
state before those companions start. A confirmed wake can then return Orion
home. Built-in YAML and service paths share the same rollback transaction; see
[Pi deployment](quickstart.md#deploy-to-the-pi).

Studio stores its pairing credentials on the desktop. It discovers the Pi service
at `/api/v2/voice/status`, sends allowlisted commands to `/api/v2/voice/request`,
and polls `/api/v2/voice/events`. Each snapshot contains at most 33 events with a
coordinator generation and increasing event IDs. The UI ignores duplicates and
resets its cursor when the generation changes. It receives status, transcripts
and timings; audio uploads and playback acknowledgement remain on the Pi.

Remote Studio access uses HTTP with a bearer token on a trusted local network.
The service's private control and observer credentials remain local. See the
[Pi quickstart](quickstart.md#prepare-a-new-pi) for installation and the
[speech worker](../speech/README.md#inference-protocol) for inference framing.

## Orion service lifecycle

`orion-voice-stack.service` starts the `orion-service` executable at boot. An OS
file lock prevents duplicate owners. The service publishes a random loopback
address and token in a private directory for the gateway to discover.

The host reads saved settings, starts the coordinator and retries a stopped
coordinator every five seconds. Repeated starts with matching configuration
reuse the coordinator. Settings changes are serialized with startup. Changing
speech or agent model settings restarts the coordinator and speech workers. The
agent executor can survive that restart when its configuration matches and no
active request is cancelled.

SIGTERM stops the coordinator, cancels its speech run and shuts down the agent
and workers. A service restart creates a fresh conversation and retains saved
memory, personality and settings. Studio's desktop backend stores pairing and
forwards requests to the Pi; an unavailable Pi produces a connection error.

## Agent and physical boundary

Qwen transcribes microphone audio locally. Codex receives confirmed command text
and any profile or memory context used for the turn. Its built-in search can
retrieve online information. Orion registers memory, lighting, mode, sleep and
alert tools. See the [tool reference](../agent/README.md#memory-and-tools).

Lighting calls validate brightness, effects and colors, then use the coordinator
and gateway's `lamp_effect` operation. The runtime stores that lamp program below
speech, scene and voice-feedback lighting. Rest darkness suppresses the output
while preserving the preference. An execution error is returned to the agent.
Mode and alert calls use the gateway's `routines` operation. Sleep requests attach
the current confirmed voice session; rest waits until its acknowledgement ends.
For a clear, immediate sleep phrase, the agent service invokes `go_to_sleep`
before returning any spoken promise. If the runtime rejects it, Orion says the
sleep request failed. Longer or conditional requests remain with Codex.
The agent cannot specify joint targets or bypass the rest lifecycle.

Tool requests must match the active Codex thread and turn. At most 16 distinct
calls execute per turn; duplicates and excess calls receive error results without
repeating an action. Unexpected interactive requests fail the turn. Orion disables
Codex shell, desktop, browser, plugin and multi-agent capabilities for its conversation.
The provider's protocol checks live in
[the Codex adapter](../agent/src/providers/codex.rs).

## Agent conversation and memory

The agent owns one ephemeral Codex conversation across wake requests and
follow-ups. Voice session IDs identify individual capture/playback turns; they
do not identify the agent conversation. Pi transport reconnects and idle
coordinator reloads can retain that conversation.

Changing the agent model, effort or executable, stopping the host, or cancelling
an active request retires the conversation. Wake-phrase barge-in stops delivery
while letting the agent request finish, preserving the conversation. Protocol
uncertainty also retires it: process exit, unreadable events, mid-turn timeout, stale/invalid calls, multiple
final answers, changed streamed speech or unsupported server requests. The next
request starts a fresh thread. Matching terminal turns retain the conversation,
including failed turns and empty final answers without streamed speech; explicit
turn-start rejections also retain it. There is no persisted thread-resume policy
or idle-time rotation. Closing the five-second follow-up window preserves context.

The agent saves memories only when requested. Entries use IDs and UTC creation
dates in a local `MEMORY.md`; writes are locked and replace the file atomically.
Retrieval uses bounded keyword matching. Selected memories are sent to Codex as
context. Studio supports memory edits and curated personality choices saved in
`SOUL.md`. Profile writes wait behind active agent requests and retire the current
conversation so later turns use the changed profile. See
[agent storage and tools](../agent/README.md#memory-and-tools) for limits and file
formats.

The coordinator saves each voice turn on the Pi under
`~/.local/share/orion/voice-stack/history/YYYY-MM-DD/`. Each private turn file
contains dated events for the recognized command, agent reply, tool calls and
results, errors, speech generation and playback timings. Audio is not saved.
Studio reads this history through the paired gateway from **Debug → Voice → View
conversation history**. Turns are shown newest first, with older turns available
through **Load older**. A coordinator restart does not erase saved turns. The
history begins when this version is installed; earlier event snapshots cannot be
reconstructed from Studio.

A search can produce one short acknowledgement while Codex continues working.
The coordinator synthesizes and plays it in the same voice session, then sends
`session.processing` to restore thinking feedback. Final speech waits for that
intermediate playback. Only final playback completion opens the follow-up window.
Memory tools are silent. Listeners without `toolFeedback` receive the final answer
without the intermediate acknowledgement.

## Response latency

Response time includes trailing silence, transcription, online agent processing,
synthesis buffering and speaker startup. Qwen and the selected TTS model stay
loaded between completed jobs. Cancelling native inference retires only the
affected worker,
which reloads for its next job. CPU thread settings are listed in
[configuration](configuration.md#pi-voice-profile).

Piper Alba generates 22,050 Hz speech. Its worker completes a sentence, resamples
it to the 24,000 Hz playback protocol, then sends chunks of at most two seconds.
The coordinator buffers at least six
seconds of audio, or the complete response when shorter, before uploading it.
Fast generation releases a reserve between six and twelve seconds. Measured slow
generation or a decoder pause requiring a larger reserve latches complete-reply
buffering, so playback cannot outrun synthesis. The coordinator retains that
reply until its final synthesis end marker, subject to the thirty-minute cap.

A first generated chunk, a first upload and audible speech are different points
in the turn. Debug exposes separate stage durations. Stages overlap, so adding
those durations does not give total response time. Runtime player start is a
software measurement; acoustic timing requires a recording at the speaker.

## Direction evidence

On V1 the listener keeps at most 30 accepted stereo observations; on V2 the
optional USB poller keeps at most 64 beam samples from the preceding three
seconds. A known side requires at least five votes and 75% agreement.
Confidence describes agreement between those observations. Microphone spacing
and channel orientation default to zero on V1. V2 direction defaults off and
requires a measured mounting offset and sign. Barge-in sessions never turn.

The age of the oldest supporting vote travels with the side after confirmation.
Time spent waiting in the command queue and homing also counts. The runtime checks
that the evidence remains younger than three seconds before starting attention.
Stale evidence leaves Orion facing home and lets the voice turn continue. Physical
calibration is required before enabling direction estimates.

## Streaming replies and timing

The Codex adapter streams speech text only from a matching thread, turn and item
explicitly marked `final_answer`. Complete sentences can reach the selected TTS
model while Codex finishes. Phase-less messages wait for final completion. They do not participate in
multiple-final or streamed-item checks. If the turn has no explicit final, the
last phase-less message is the answer. Two distinct explicit final items retire
the conversation. Model
commentary and citation markers are removed from spoken output. The final text
must preserve any prefix
already emitted; a mismatch cancels the turn and retires the agent conversation.
The base prompt requests a short opening sentence and prose for listening, without
markdown, bullet symbols, tables or headings. There is no output truncation or
word/sentence quota. The personality's “Keep it brief” habit controls brevity, and
the 64 KiB streamed-answer bound guards runaway generation.

Successive sentences use one TTS worker and one runtime speech run. The selected
voice is captured when the response starts. Bounded PCM queues and explicit job IDs,
chunk sequences and end markers prevent mixed or incomplete responses from being
accepted. Text delivery queues independently of inference, bounded by the agent's
64 KiB streamed-answer and 2 MiB event guards, so slow TTS does not hold the Codex
turn open. Every sentence or unpunctuated tail is divided into pieces of at most
160 Unicode characters before inference, preferring words and sentence ends. Each
piece gets its own 240-second deadline. Piper also bounds its native decoder
inputs, retaining the 120-second per-input audio sanity guard.

The service's 135-second RPC read, 130-second dispatch and 140-second remote
request deadlines cover control/profile requests, not voice playback. The agent's
120-second deadline covers a Codex turn and tool results; final synthesis and
playback continue independently after it finishes. See [agent conversation](#agent-conversation-and-memory)
for failure recovery. Every turn includes UTC time, local time with an offset and
a discovered IANA zone. If no zone is known, both the context and `list_alerts`
say `unknown; use the local UTC offset above`. Today's offset is known even when
the zone is not; a future date's offset may require clarification.

Gateway validation reasons reach the agent through `error.message` or the
runtime's `result.error`, with an HTTP status fallback and credential/URL redaction.
`get_lighting` reads manual brightness in percent, effect and RGBW colors; null
means no manual override. `set_lighting` explains that scenes/speech reject changes
and character startup clears a manual light.

The coordinator uploads mono 24 kHz PCM16 WAV through the gateway.
`POST /api/v2/speech/stream` creates a runtime run,
`/api/v2/speech/{run}/chunks/{sequence}` appends audio, and
`/api/v2/speech/{run}/end` declares the final sequence. The complete-WAV endpoint
also remains available.

The required startup reserve is the larger of six seconds or twice the longest
measured generation step plus two seconds. Once six seconds have accumulated,
generation taking more than 75% of audio duration, or a required reserve above
twelve seconds, latches complete buffering. That reply uploads only after the
final synthesis end marker, even if later chunks are faster. Fast generation
continues to release between six and twelve seconds, with at most one chunk of
overshoot. Short replies finish generation before upload.

Uploads wait while `buffered_ms` exceeds sixteen seconds, limiting accepted audio
to about eighteen seconds ahead of playing audio. Bounded synthesis channels
apply backpressure while the player catches up. A complete buffered reply bypasses
this wait while the runtime remains `queued`: no playback can drain that reserve,
and withholding more chunks would eventually hit upload-idle expiry. Its remaining
chunks and end marker can therefore arrive in a burst before playback readiness.
A complete reply that begins a queued burst finishes it even if playback starts
midway: a large accumulated lead could take more than ten seconds to drain to the
pacing threshold. Replies uploaded to an already-playing runtime retain normal
sixteen-second pacing. Under normal paced playback each at-most-two-second upload
renews the runtime's existing ten-second upload-idle guard well before expiry.
The guard remains active until the end marker; after that marker, upload-idle
expiry no longer applies. Per-upload WAV bounds and two-second chunk bounds remain.

The runtime computes smoothed 20 ms energy frames incrementally, replacing a
provisional partial frame when the next chunk arrives. Quiet regions and phrase
peaks use this compact energy history; raw PCM leaves memory after the player
accepts it. Stream chunk files are deleted after acceptance, not read by the
player. Character planning requires absolute frame indices, so analysis history
remains until completion. Streams accept thirty minutes of audio, then fail with
an explicit sanity-limit reason. The coordinator cancels playback, publishes the
failure, and supplies the reason to the next model turn. No extra inference starts
on failure and no successful completion conceals truncated speech.

At 24 kHz mono PCM16, raw audio costs 2.88 MB per minute. The removed retained
copy and full decoded-sample allocation cost 2.88 MB and 11.52 MB per minute,
respectively. Incremental RMS history costs 24 KB per minute plus quiet/peak
indices; its worst-case logical landmark storage is below 50 KB per minute.
Fast-path PCM queues and startup reserve have fixed bounds. A complete-buffered
reply retains 2.88 MB per minute in the coordinator, up to 86.4 MB at thirty
minutes. The coordinator checks this cap before retaining each chunk and reports
the same explicit runtime failure reason if another chunk would exceed it. A
queued complete-reply burst or an unpaced external uploader can also queue up to
86.4 MB at the runtime. Complete queued bursts finish uploading without pacing;
other replies retain the sixteen-second upload threshold.
These figures describe payloads, excluding allocator capacity, in-flight chunks
and process/library overhead.

The runtime prebuffers two seconds, or a shorter complete reply, and feeds one
`aplay` process continuously. Each chunk contains at most two seconds of audio.
Upload completion is followed by actual player completion before the listener is
acknowledged. Out-of-order chunks, upload stalls, cancellation and buffer
exhaustion terminate the run. A later generation slowdown can still exhaust a
buffer chosen from earlier timings. The coordinator replaces a total playback
deadline with a 20-second stall guard: state changes, accepted chunks and advancing
software playback position reset it. Waiting for synthesis with an exhausted buffer
does not consume the stall budget. `buffered_ms` estimates playback position from
elapsed time rather than measuring ALSA consumption, so a hung player is detected
after the expected audio end plus the stall interval. Physical audio completion
still comes from the player.

The runtime analyzes received audio and extends the character's existing motion
run at gesture boundaries. Extension preserves commanded position and velocity,
the anchor and performed gesture history. The stream end marker revises the
remaining plan, and terminal playback starts or continues a final settle. See
[speech performance policy](motion-reference.md#speech-performance-policy) for the
motion policy.

Coordinator logs include `speech.buffer_ready` and `speech.chunk`; runtime logs
include `speech.chunk_received`, `speech.stream_end`, `speech.terminal` and motion
finalization events. `buffered_ms` estimates received duration minus software
playback elapsed. It can be negative and does not measure the ALSA hardware
buffer. These timing logs contain IDs and durations, while Studio's conversation
history also exposes transcripts and agent actions. Use the [service logs](quickstart.md#logs-and-recovery)
to investigate a failed turn.

## Local interaction feedback

Wake, verification, endpoint and processing events carry the voice session ID.
The runtime rejects stale transitions and bounds their lifetime. A candidate
plays the existing wake chime once and starts amber, teal and purple
acknowledgement pulses, each lasting 300 ms. The body reaction remains unchanged
until Qwen confirmation. The pulse clock starts at the candidate; Qwen does not
replay it after a slow verification or fallback.
Afterward, capture uses the dim listening light and endpointed commands use
thinking feedback. An unconfirmed endpoint remains silent.

Thinking uses one entry cue, breathing light and a restrained head tilt with
supporting shoulder and elbow movement. Repeated thinking notifications preserve
the current gesture and timing. This prevents the transcription-to-agent
transition from replaying the opening movement. Speech begins from the commanded
thinking position and velocity when it takes over.

If processing becomes unavailable after confirmation, the listener requests
`error_muted` once and returns to wake detection. Unconfirmed rejection,
cancellation and failure clear the candidate light without additional feedback.
During descent, waking or a rest fault, immediate voice reactions are suppressed.
Mechanical rest permits only the candidate's chime and brief light pulse. Home
completion restores the latest eligible reaction. Capture stays
open through cues, so speaker pickup must be checked on the assembled robot.

## Alert interruption

A ringing timer or alarm takes the speaker from an active voice reply. The
listener retires that voice session and sends `session.interrupted`, allowing the
coordinator to cancel inference and playback. Late messages from the retired
session cannot replace the alert. Rustpotter continues listening during the sound;
a detection sends a local `routines` stop request and consumes the wake phrase.
Qwen and Codex do not participate in dismissal. See
[timer and alarm behavior](system-architecture.md#timers-and-alarms).
