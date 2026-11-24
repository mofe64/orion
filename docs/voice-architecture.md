# Orion voice architecture

Orion's voice pipeline splits capture, coordination, AI models and playback across dedicated processes so audio never drops while models think or reply. Speech recognition and speech synthesis run locally on the Pi; only Codex replies and web search go online. See [system boundaries](system-architecture.md#system-boundary).

## Audio and control flow

```text
microphone
  → continuous openWakeWord detector
  → Qwen wake-prefix check
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

## Capture ownership and session lifecycle

`orion-listener` opens the microphone at startup unless microphone mute is saved. One authenticated local connection owns voice processing, while separate control connections can read or change mute.

V1 captures stereo audio for coarse left/right direction and downmixes to mono for recognition; V2 captures the XVF3800 board's processed channel. The listener keeps 3 seconds of rolling pre-roll in memory so the start of `"Hey Orion"` is never cut off. Each wake candidate receives a fresh session ID—cancellation, mute, disconnect or expiry discards its command buffer, while rejecting a false alarm leaves the rolling pre-roll intact.

### Wake detection and confirmation

`openWakeWord` scores every 20 ms frame continuously across listening, processing, playback, alarms and follow-up windows, suppressing repeat triggers for 2 seconds after each hit (only a capture gap resets its history).

When it fires, Qwen transcribes a prefix covering the 3-second pre-roll plus the next 300 ms while full command recording continues in parallel. If `"Hey Orion"` appears in the first eight words, Orion plays the wake cue, pulses its light and starts listening animation; otherwise the turn cancels invisibly without interrupting anything.

During wake and command capture, Orion ducks music to 10% volume (and to 25% while Orion is speaking).

### Command transcription

Once you stop speaking, Qwen transcribes the complete recording (starting with the 3-second pre-roll). Because the prefix already confirmed the wake phrase, the full transcript cannot reject the turn later; the coordinator simply strips the wake phrase and forwards the rest as your command. If you only said `"Hey Orion"` with no command, Orion waits for follow-up speech instead.

Before sending audio to Qwen, the listener shortens silences longer than 600 ms down to 320 ms while preserving all speech and pause edges in order. Capture stops after 6 seconds of complete silence or at the 15-second limit (18 seconds with pre-roll); in either case, Orion plays its `"didn't catch that"` cue and makes an unsure gesture rather than dropping the turn silently.

Passing `--wake-engine rustpotter` switches back to the earlier detector (with an optional acoustic verifier).

### Speech boundaries

Silero VAD (`voice activity detection`) tracks speech probability across windows using separate start (`0.5`) and stop (`0.35`) thresholds so brief wobbles do not chop sentences apart, ending capture after 1.2 seconds of trailing silence. The standalone listener can fall back to an energy detector when no VAD model is configured.

## Confirmed waking

An accepted wake prefix wakes a resting lamp back to `home` (or waits if it is mid-descent) while keeping the microphone recording. Once homing completes, the runtime applies the latest listening or thinking state, turns toward the selected speaker if vision found one, and holds reply playback until that attention move finishes. Duplicate confirmations cannot extend the same session's inactivity timer, and Stop, disable or maintenance startup still prevent automatic recovery.

## Follow-up conversation

After reply playback succeeds, the listener discards initial speaker echo, waits for quiet, and opens a 5-second follow-up window (indicated by a teal pulse) where you can reply without saying `"Hey Orion"` again. Each follow-up receives a new session ID linked to the finished turn, requires at least 400 ms of speech, and ignores empty transcripts, single words or a bare wake phrase. Failed or cancelled playback skips follow-up and returns straight to wake listening.

Keepalive messages extend active processing or playback deadlines without repeating visual/audio cues, and an absent coordinator loses ownership on timeout.

## Wake-phrase interruption

Because V2 uses the XVF3800 board's hardware echo cancellation (with no extra software AEC), `openWakeWord` can still hear `"Hey Orion"` while Orion is speaking. Any wake detection during speech or processing stops the current reply immediately and starts a fresh prefix check (`"Hey Orion, stop"` ends the reply quietly; alarm dismissal takes priority over starting a new conversation). Interruptions skip the normal speaker attention turn.

## Agent conversation and memory

The `orion-agent` service outlives individual turns and reuses Codex provider context across accepted turns within a fixed 24-hour conversation window (follow-ups do not extend that 24-hour boundary, and rejected or empty wakes create no turn). If protocol state becomes uncertain, a fresh Codex session receives bounded text recovery context.

Reply generation and delivery track separate outcomes so an interrupted reply does not pretend to have been heard. Automatic memory extraction runs on retired conversations using receipts and edit cutoffs so deleting a memory prevents it from coming back later (see [memory storage](../agent/README.md#memory-and-profile-storage)). The behavior journal records observations and actions without storing camera pixels, microphone audio or persistent people IDs.

## Direction and speaker selection

V1 estimates coarse left/right audio direction from stereo microphone timing. V2 asks `orion-vision` once per confirmed wake for the speaker whose mouth moved most during the wake phrase, and follows that track ID through the conversation (only in `Idle` or `Faces` tracking; `Off`, `Hands`, privacy off or unavailable vision suppress attention turns, and missing speakers are never replaced with bystanders). USB direction settings are diagnostic only and never steer V2 attention.

## Streaming replies and timing

The coordinator splits Codex's final reply at sentence/word boundaries into bounded Piper jobs and validates chunk order, IDs and PCM format before uploading to `oriond`. While Piper stays comfortably faster than real time, Orion builds a startup reserve and streams chunks smoothly; if Piper falls behind, the coordinator switches to buffering the whole reply so playback never stutters mid-sentence.

Uploads apply backpressure once enough audio is queued ahead of playback, while fully buffered replies keep uploading to deliver their end marker. Playback progress reflects `oriond`'s software position rather than an acoustic sensor, and oversized replies are rejected rather than truncated.

## Tool feedback and cancellation

When Codex runs a web search or slow tool, Orion speaks a brief acknowledgement run first, then waits for it before delivering the final reply (only the final reply opens a follow-up window). Robot tools return typed status and failure reasons from `oriond` before Codex confirms an action.

Cancelled or failed turns retire only the affected speech worker when needed, clear old uploads and playback, and ignore stale events from expired sessions so late results can never resurrect a dead turn.

## Scheduled announcements

Scheduled reminders and announcements run only when ordinary voice work is idle, wait for `oriond` ownership, and can be interrupted by `"Hey Orion"`. Completed playback is recorded before sending the separate completion acknowledgement so network retries never replay spoken reminders. Morning briefings after alarm dismissal run as their own agent turn without creating user transcripts or opening a follow-up window.

## Look tool contract

Explicit visual questions (`"what am I holding?"`) use the `look` tool to fetch one in-memory camera snapshot through the authenticated gateway and pass it to Codex. Camera privacy, rest and viewing permissions still apply; images are never saved to disk or written into history/logs, and image access failures return an error rather than guessing. Autonomous background behaviors never upload images (except Demo mode remarks).

See [speech workers](../speech/README.md), [listener setup](../voice/README.md) and [configuration](configuration.md).

## Private recordings

Orion saves no wake audio by default. Setting `ORION_WAKE_CLIPS_DIR` records bounded prefix WAV clips on a background thread for debugging; see [listener troubleshooting](../voice/README.md#troubleshooting). On the Pi, `orion-record-voice` records private training samples and restores the listener when finished; see [record your voice](wake-word-training.md#record-your-voice).
