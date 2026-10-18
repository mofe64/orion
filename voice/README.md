# Orion voice listener

The listener owns microphone capture, wake detection and voice-session boundaries. It runs in the active release's Python 3.12 environment. The coordinator connects through an authenticated WebSocket.

## Setup

Use the [Pi installer](../docs/quickstart.md) for the complete listener and models. V1 uses the audio HAT. V2 uses the XVF3800 USB array.

The installed service supplies an explicit wake model path and `--no-verifier`. `--local-processor` reserves processing ownership for a loopback connection. Other authenticated connections can read or change mute.

## Capture and sessions

V1 captures synchronized stereo PCM16 at 16 kHz. It retains stereo for direction estimation and downmixes for recognition. V2 uses the selected processed USB channel.

The listener keeps pre-roll and command audio in memory. Pre-roll is audio retained from before wake detection. Mute, cancellation and disconnect clear session buffers.

Rustpotter proposes a candidate. By default, Qwen checks a short wake prefix. The wake cue and light pulse start only after Qwen accepts the phrase. `--verifier-dir` turns on the acoustic verifier instead. Rejection discards candidate audio while preserving rolling pre-roll.

Qwen checks the complete utterance before the agent receives a command. Recordings that reach the capture limit do not become agent requests. See [voice sessions](../docs/voice-architecture.md#capture-ownership-and-session-lifecycle).

## Playback interruption

V2 can accept an acoustically verified “Hey Orion” during playback. Acceptance cancels the old reply and starts a new session. Rejection leaves playback unchanged.

This operation requires a healthy acoustic verifier. It uses XVF3800 hardware echo cancellation. Orion adds no software echo canceller. V1 suppresses wake detection during ordinary reply playback.

Alarm dismissal takes priority over conversation interruption. After normal playback, the listener waits for quiet before opening the follow-up window.

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

Wake audio saving requires the explicit `ORION_WAKE_DEBUG_DIR` setting. These recordings contain private microphone audio. The listener does not save them by default.

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
