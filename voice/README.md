# Orion voice listener

The listener owns microphone capture, wake detection and voice-session boundaries. It runs in the active release's Python 3.12 environment. The coordinator connects through an authenticated WebSocket.

## Setup

Use the [Pi installer](../docs/quickstart.md) for the complete listener and models. V1 uses the audio HAT. V2 uses the XVF3800 USB array.

The installed service uses openWakeWord and supplies its model directory with `--verifier-dir`. `--local-processor` reserves processing ownership for a loopback connection. Other authenticated connections can read or change mute.

## Capture and sessions

V1 captures synchronized stereo PCM16 at 16 kHz. It retains stereo for direction estimation and downmixes for recognition. V2 uses the selected processed USB channel.

The listener keeps pre-roll and command audio in memory. Pre-roll is audio retained from before wake detection. Mute, cancellation and disconnect clear command buffers. Wake rejection retains the rolling pre-roll.

openWakeWord scores every 20 ms frame in every phase. One scorer keeps its history across turns and pauses detection for two seconds after a detection.
Only a capture gap resets the scorer. Model hashes, the threshold and startup frames come from `models/verifier/config.json`.

Qwen checks one prefix containing three seconds before detection and 300 ms after detection. Acceptance permits the wake cue and light pulse.
Rejection cancels the turn immediately. Pre-roll and detector state remain available for the next wake.

Qwen also transcribes the complete recording to obtain the command. An accepted prefix remains accepted when that transcript omits the wake phrase.
Capture ends after silence or at the 15-second limit. Recordings that reach the limit do not become agent requests.

## Playback interruption

A wake detection during speech or processing cancels the old turn and starts a new prefix check. “Hey Orion, stop” stops speech.
Alarm dismissal takes priority. After normal playback, the listener waits for quiet before opening the follow-up window.

V2 uses XVF3800 hardware echo cancellation. Orion adds no software echo canceller.
`--wake-engine rustpotter` selects the previous detector. Its optional acoustic verifier uses the same model directory.

## Settings

`--capture-channels` and `--processed-channel` select the V2 capture layout. `--hardware v1` selects HAT routing. `--hardware v2` skips HAT mixer setup.

V2 confirmed sessions request visual speaker selection once. The selected track can continue through the conversation. Camera privacy, disabled gaze or unavailable vision suppresses this attention movement.

Microphone mute persists in `~/.config/orion/microphone.json`. The listener configures capture before reporting readiness. V1 reapplies gain after capture starts.

The [configuration reference](../docs/configuration.md#microphone-listener) defines environment settings. Wake model changes follow the [training procedure](../docs/wake-word-training.md).

## Troubleshooting

1. Read listener and host status.

   ```bash
   systemctl --no-pager --full status orion-listener orion-voice-stack
   ```

2. Read recent voice messages.

   ```bash
   journalctl -u orion-listener -u orion-voice-stack -n 60 --no-pager
   ```

Each detection produces a `voice.wake_detection` JSON line with score, phase and session ID.
The coordinator reports `voice.wake_outcome` with acceptance and Qwen time. These outcome lines contain no transcript text.

Set `ORION_WAKE_CLIPS_DIR` in the listener service environment to save accepted and rejected prefix WAV files.
The writer uses a separate thread. `clips.jsonl` links each clip to its session and outcome.
The directory holds at most 200 files and 100 MB. Old clips are removed first.
Audio saving stays off when the variable is absent. These files contain private microphone audio and stay on the lamp.

`ORION_WAKE_DEBUG_DIR` enables the separate Rustpotter acoustic-verifier diagnostics.
The [recording procedure](../docs/wake-word-training.md#record-your-voice) uses `orion-record-voice` for private training samples.

## Development

Run these steps from the repository root.

`oriond` keeps the speaker open. Stop `oriond` before using `orion_voice.xvf_measure record --play` for direct speaker measurements.
Start `oriond` again after the measurement. Speaker measurements do not play while `oriond` runs.

1. Install locked dependencies.

   ```bash
   uv sync --project voice --locked --managed-python
   ```

2. Run listener tests.

   ```bash
   uv run --project voice --locked --managed-python python -m unittest discover -s voice/tests -v
   ```

The native Rustpotter package has its own `voice/rustpotter_native/pyproject.toml` and lock file. Release builds compile it for ARM64.
