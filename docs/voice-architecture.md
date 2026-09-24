# Orion voice architecture

Orion's Pi runs microphone capture, wake detection, speech recognition, speech
synthesis and the agent coordinator. Rustpotter detects a possible wake phrase,
Silero finds the end of speech, Qwen3-ASR transcribes, and Piper Alba Medium
produces the reply. Codex App Server runs on the Pi
and uses online model inference. Studio provides settings and observation through
the gateway.

## Audio and control flow

```text
ReSpeaker stereo capture
  -> direction observation and mono downmix
  -> silent Rustpotter wake candidate
  -> short Qwen wake verification while command capture continues
  -> Silero endpoint -> Qwen transcription of the complete recording
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
before downmixing to mono and retains three seconds of audio in memory. When
Rustpotter detects a candidate, the listener assigns a random session ID and
registers a silent runtime session before notifying the coordinator.

With the negotiated `wakePrefix` capability, the listener sends up to two seconds
before detection plus 200 ms afterward for Qwen verification. Capture continues
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

At mechanical rest, the wake candidate remains silent, still and dark. Qwen
confirmation plays the existing wake chime and starts the return home. The
rest lifecycle keeps the light off until home completes.

The coordinator sends `wake.verified` after a successful prefix or
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

Processing and playback suppress additional wake triggers. After the runtime
reports successful playback completion, the coordinator acknowledges it to the
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

Processing has a 120-second session lease; entering playback grants 180 seconds.
Disconnect, mute, cancellation and timeout clear the current session. The
coordinator reconnects automatically. Protocol capabilities allow older peers to
confirm the complete utterance or receive a single response without a follow-up
window.

Acoustic echo cancellation and interruption during playback are not implemented.
Speak after the teal invitation appears. Sustained noise or delayed echo can
still trigger an unwanted follow-up, so microphone and speaker behavior require
physical checks.

## Transport and deployment

The onboard coordinator reads the Pi token file and connects to the listener and
gateway through loopback. `--local-processor` restricts processing ownership to a
local connection. Private stdin/stdout pipes carry speech jobs and the Codex App
Server protocol; bounded Rust channels connect the coordinator to the agent.

Studio stores its pairing credentials on the desktop. It discovers the Pi service
at `/api/v2/voice/status`, sends allowlisted commands to `/api/v2/voice/request`,
and polls `/api/v2/voice/events`. Each snapshot contains at most 33 events with a
coordinator generation and increasing event IDs. The UI ignores duplicates and
resets its cursor when the generation changes. It receives status, transcripts
and timings; audio uploads and playback acknowledgement remain on the Pi.

Remote Studio access uses HTTP with a bearer token on a trusted local network.
The service's private control and observer credentials remain local. See the
[Pi quickstart](quickstart.md#pi-local-voice-and-agent) for installation and the
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
The agent cannot specify joint targets or bypass the rest lifecycle.

Tool requests must match the active Codex thread and turn and are limited to
16 per turn. Unexpected interactive requests fail the turn. Orion disables Codex
shell, desktop, browser, plugin and multi-agent capabilities for its conversation.
The provider's protocol checks live in
[the Codex adapter](../agent/src/providers/codex.rs).

## Agent conversation and memory

The agent owns one ephemeral Codex conversation across wake requests and
follow-ups. Voice session IDs identify individual capture/playback turns; they
do not identify the agent conversation. Pi transport reconnects and idle
coordinator reloads can retain that conversation.

Changing the agent model, effort or executable, stopping the host, or cancelling
or failing an active agent request retires the conversation. The next request
starts a fresh thread. There is no persisted thread-resume policy or idle-time
rotation. Closing the five-second follow-up window preserves conversation context.

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
It can retain a slower reply until completion to prevent playback gaps. The
buffering decision follows generation speed for the current response, rather
than a fixed model choice.

A first generated chunk, a first upload and audible speech are different points
in the turn. Debug exposes separate stage durations. Stages overlap, so adding
those durations does not give total response time. Runtime player start is a
software measurement; acoustic timing requires a recording at the speaker.

## Direction evidence

The listener keeps at most 30 accepted stereo observations from the preceding
three seconds. A known side requires at least five votes and 75% agreement.
Confidence describes agreement between those observations. Microphone spacing
and channel orientation default to zero, which disables directional attention.

The age of the oldest supporting vote travels with the side after confirmation.
Time spent waiting in the command queue and homing also counts. The runtime checks
that the evidence remains younger than three seconds before starting attention.
Stale evidence leaves Orion facing home and lets the voice turn continue. Physical
calibration is required before enabling direction estimates.

## Streaming replies and timing

The Codex adapter streams speech text only from a matching thread, turn and item
explicitly marked `final_answer`. Complete sentences can reach the selected TTS
model while Codex finishes. Unknown phases wait for final completion. Model
commentary and citation markers are removed from spoken output. The final text
must preserve any prefix
already emitted; a mismatch cancels the turn. Spoken output is capped at
800 Unicode characters plus an ellipsis.

Successive sentences use one TTS worker and one runtime speech run. The selected
voice is captured when the response starts. Bounded queues and explicit job IDs,
chunk sequences and end markers prevent mixed or incomplete responses from being
accepted. A speech job has a 240-second host deadline, while the listener's lease
also bounds the overall turn.

The coordinator uploads mono 24 kHz PCM16 WAV through the gateway.
`POST /api/v2/speech/stream` creates a runtime run,
`/api/v2/speech/{run}/chunks/{sequence}` appends audio, and
`/api/v2/speech/{run}/end` declares the final sequence. The complete-WAV endpoint
also remains available.

The startup reserve is the larger of six seconds of audio or twice the longest
measured generation step plus two seconds. Once six seconds have accumulated,
generation taking more than 75% of the audio duration, or a reserve above twelve
seconds, makes the coordinator buffer the complete response. Short replies finish generation
before upload. The 120-second audio limit bounds the coordinator's PCM buffer to
about 5.8 MB; it does not extend the voice-session deadline.

The runtime prebuffers two seconds, or a shorter complete reply, and feeds one
`aplay` process continuously. Each chunk contains at most two seconds of audio.
Upload completion is followed by actual player completion before the listener is
acknowledged. Out-of-order chunks, upload stalls, cancellation and buffer
exhaustion terminate the run. A later generation slowdown can still exhaust a
buffer chosen from earlier timings.

The runtime analyzes received audio and extends the character's existing motion
run at gesture boundaries. Extension preserves commanded position and velocity,
the anchor and performed gesture history. The stream end marker revises the
remaining plan, and terminal playback starts or continues a final settle. See
[character animation](character-animation.md#speech-driven-animation) for the
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
stays silent and leaves the existing light and character reaction unchanged.
Qwen confirmation requests the existing wake chime once and starts the unchanged
amber, teal and purple acknowledgement pulses, each lasting 300 ms. The pulse
clock starts at confirmation and survives an immediate endpoint or follow-up.
Afterward, capture uses the dim listening light and endpointed commands use
thinking feedback. An unconfirmed endpoint remains silent.

Thinking uses one entry cue, breathing light and a restrained head tilt with
supporting shoulder and elbow movement. Repeated thinking notifications preserve
the current gesture and timing. This prevents the transcription-to-agent
transition from replaying the opening movement. Speech begins from the commanded
thinking position and velocity when it takes over.

If processing becomes unavailable after confirmation, the listener requests
`error_muted` once and returns to wake detection. Unconfirmed rejection,
cancellation and failure produce no feedback. During descent, waking or a rest
fault, immediate voice reactions are suppressed. Mechanical rest permits only
the confirmed wake's chime. Home completion restores the latest eligible reaction. Capture stays
open through cues, so speaker pickup must be checked on the assembled robot.

## Alert interruption

A ringing timer or alarm takes the speaker from an active voice reply. The
listener retires that voice session and sends `session.interrupted`, allowing the
coordinator to cancel inference and playback. Late messages from the retired
session cannot replace the alert. Rustpotter continues listening during the sound;
a detection sends a local `routines` stop request and consumes the wake phrase.
Qwen and Codex do not participate in dismissal. See
[timer and alarm behavior](system-architecture.md#timers-and-alarms).
