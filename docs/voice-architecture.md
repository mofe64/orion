# Orion voice architecture

The listener captures speech. The coordinator runs recognition, agent requests and synthesis. The runtime plays audio and moves the lamp. Studio observes and configures the Pi through the gateway.

## Audio and control flow

```text
microphone
  → Rustpotter candidate
  → acoustic phrase check
  → Silero speech boundary
  → Qwen complete-command transcription
  → Codex agent
  → Piper synthesis
  → coordinator audio buffer
  → gateway upload
  → runtime playback and animation
  → quiet guard
  → follow-up window
```

Speech recognition and synthesis run locally. Codex App Server uses online inference. See [system boundaries](system-architecture.md#system-boundary).

## Capture ownership and session lifecycle

The listener opens capture unless microphone mute is saved. One authenticated local connection owns processing. Separate control connections can change mute.

V1 uses stereo frames for direction and a mono downmix for recognition. V2 uses the XVF3800 processed channel. The listener retains three seconds of pre-roll in memory. Pre-roll preserves speech from before wake detection.

Each candidate receives a fresh session ID. Cancellation, mute, disconnect and expiry discard its command buffers. Rejected candidates do not reach the agent.

### Acoustic wake verification

Rustpotter proposes “Hey Orion” candidates. By default, Qwen transcribes a short wake prefix and checks it for the phrase. Acceptance permits the wake cue, light pulse and confirmed-wake sequence. Nothing is shown or played before acceptance.

Rejection discards candidate audio while preserving rolling pre-roll and detector history. It does not restart capture. The optional acoustic verifier (`--verifier-dir`) replaces the Qwen prefix check when loaded.

Full command capture continues while Qwen checks the prefix. A slow or missing acoustic verifier disables playback interruption.

### Complete command checks

Qwen transcribes the complete utterance before dispatch. Prefix confirmation alone cannot supply agent command text. A wake-only utterance opens another capture opportunity.

Recognition receives speech with pauses longer than 600 ms shortened to 320 ms. The listener keeps both ends of each pause and all speech frames in their original order.

The command clip keeps 2 seconds from before wake detection. Capture permits 30 seconds after wake detection, or 32 seconds including pre-roll. A recording that reaches the limit is rejected. Empty commands also stop before agent dispatch.

### Speech boundaries

Silero classifies speech with state carried across audio windows. Different enter and leave thresholds reduce repeated switching. The managed listener ends capture after its configured silence interval.

The standalone listener can use an energy detector without a VAD model. VAD means voice activity detection. The installed profile supplies Silero.

## Confirmed waking

An accepted confirmation can start the return from rest to home. Confirmation during descent waits for descent to finish. The listener keeps capturing while the body moves.

After home completes, the runtime applies the latest listening or thinking state. It checks speaker selection before an attention turn. Reply audio waits until this movement finishes.

Duplicate confirmations cannot extend the same session's inactivity deadline. Stale sessions cannot confirm, renew or move the lamp. Stop, disable and maintenance controls keep their normal recovery requirements.

## Follow-up conversation

After successful playback, the listener discards initial echo and waits for quiet. It then opens a five-second command window without another wake phrase. A teal pulse indicates that window.

A follow-up receives a new session ID linked to the completed turn. The coordinator checks that link before recognition. Failed or cancelled playback returns to ordinary wake detection.

Keepalive messages extend an active processing or playback deadline. They do not repeat phase changes or feedback. An absent owner eventually loses its session.

## Wake-phrase interruption

V2 runs wake detection during playback only with a healthy acoustic verifier. Candidate detection does not stop playback. Acceptance cancels the old session and starts a fresh verified session from pre-roll.

Rejected candidates leave the reply unchanged. The Qwen-prefix fallback cannot authorize playback interruption. V1 suppresses wake detection during ordinary playback.

V2 uses the XVF3800's hardware echo cancellation. It adds no software echo canceller. Interruption sessions suppress speaker attention. Alarm dismissal takes priority over a new conversation.

## Agent conversation and memory

The agent service outlives individual speech jobs. Completed turns can reuse Codex provider context. Protocol uncertainty retires that provider. A replacement receives bounded text context.

Accepted turns share a logical conversation for 24 hours from its first accepted turn. Follow-ups do not extend that fixed window. Rejected or empty wakes create no turn.

Reply generation and delivery have separate outcomes. A generated reply can still fail or be interrupted during playback. Memory extraction uses retired conversations and independent proposal review.

Explicit memory edits set a cutoff for recovered context. Automatic receipts prevent duplicate extraction and reintroducing deleted entries. See [agent storage](../agent/README.md#memory-and-profile-storage).

The behavior journal stores runtime observations and actions. It excludes camera pixels and microphone audio. It does not identify people across sessions.

## Direction and speaker selection

V1 can use coarse left/right audio direction with configured microphone geometry. V2 confirmed sessions ask vision for one speaker selection. The selection includes a track ID for conversation following.

Tracking Idle or Faces can supply a speaker. Off, Hands, camera privacy and unavailable vision suppress V2 attention turns. USB direction settings remain diagnostic inputs rather than wake steering.

The selected speaker can remain the conversation target between replies. Missing speakers do not authorize substitution with a bystander. Speech movement retains priority.

## Streaming replies and timing

The coordinator splits final text into bounded synthesis jobs. Piper produces ordered PCM chunks and an explicit end marker. The coordinator validates IDs, sequence numbers, audio format and limits.

Fast synthesis uses a startup reserve. Slow synthesis selects complete buffering for the rest of that reply. Once selected, complete buffering remains active even if later chunks arrive faster.

Uploads pause when sufficient audio is queued ahead of playback. Complete buffered replies can continue uploading to deliver their end marker. This prevents an unfinished upload from expiring while queued audio waits.

Playback progress is a software position, not an independent acoustic measurement. A first generated chunk does not mean the speaker has started. Runtime stream limits reject oversized replies instead of truncating them.

## Tool feedback and cancellation

A search acknowledgement uses a separate speech run within the active session. The final reply waits for it. Only final playback opens follow-up listening.

Robot tools return typed requests through the coordinator and gateway. The runtime decides whether execution is permitted. Failed actions return their reason before the agent confirms a result.

Cancellation retires only the affected speech worker when necessary. It cancels old uploads and playback ownership. Late results cannot revive an expired session.

## Scheduled announcements

The listener admits announcements only when ordinary voice work is idle. The coordinator waits for runtime ownership before synthesis and upload. A wake can interrupt an announcement.

Completed playback is recorded before its separate acknowledgement. Retrying acknowledgement cannot replay delivered speech. Delivery failure leaves the runtime's retry policy in control.

Briefings use a separate agent request after alarm dismissal. They do not create a user transcript or open a conversation window.

## Look tool contract

An explicit visual question can request one camera snapshot. The coordinator obtains it through the authenticated gateway and sends it to Codex image input.

Camera privacy, rest and viewing permission still apply. Images stay in memory on the Pi. They do not enter tool text, history or logs. An image access error returns a failure instead of invented visual content.

Autonomous observations do not authorize image upload. General conversation does not capture a snapshot automatically.

See [speech workers](../speech/README.md), [listener setup](../voice/README.md) and [configuration](configuration.md).
