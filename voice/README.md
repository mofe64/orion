# Orion local voice runtime

`voice/` contains Orion's local speech processes. It has two narrow runtime
boundaries:

```text
agent/oriond -> /tmp/orion-tts.sock -> Piper Ryan Medium -> temporary WAV
microphones -> Sherpa keyword spotter -> /tmp/orion-wake.sock -> wake event
```

Piper owns text-to-speech model inference. `oriond` remains the only owner of
physical playback, so generated speech keeps the same speech IDs, status,
`--wait`, cancellation, and ALSA lifecycle as cues and scenes.

The wake worker owns transient 16 kHz mono microphone capture and detects the
phrase `HEY ORION` locally. It publishes JSON-line events and never writes
microphone audio to disk. The future agent runtime will subscribe to those
events. Speech-to-text is the next voice slice and is not implemented here.

## Install the Python environment

Orion uses its existing Python 3.11 environment because the Pi's system Python
is newer than the versions currently selected for the voice stack:

```bash
cd /home/mofe/dev/orion

/home/mofe/.local/bin/uv sync \
  --project voice \
  --python 3.11
```

The pinned model runtimes are Piper 1.7.0 and Sherpa ONNX 1.13.6. Orion also
declares Click, SentencePiece, and pypinyin directly because Sherpa's keyword
tokenization path imports them without declaring them itself. `uv sync`
removes packages that are no longer declared.

## Install the selected models

Run the repeatable model installer:

```bash
voice/install-models.sh
```

It performs four bounded operations:

1. Downloads `en_US-ryan-medium`, Orion's selected production voice.
2. Downloads Sherpa's English 3.3-million-parameter GigaSpeech keyword model.
3. Generates the model-specific BPE tokens for `HEY ORION`.
4. Removes other top-level Piper voice files while retaining the wake model.

The resulting runtime data is ignored by Git:

```text
voice/models/en_US-ryan-medium.onnx
voice/models/en_US-ryan-medium.onnx.json
voice/models/wake/sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01/
```

To repeat only the selected-voice cleanup:

```bash
voice/cleanup-voices.sh
```

The cleanup refuses to run unless both Ryan Medium files are already present.
It removes only `.onnx` and `.onnx.json` files directly inside `voice/models/`;
it does not touch nested wake-word or future speech-recognition models.

## Run text to speech

Terminal 1 loads Ryan Medium once and owns TTS inference:

```bash
cd /home/mofe/dev/orion
voice/.venv/bin/orion-voice tts-worker
```

Wait for:

```text
orion-tts: ready on /tmp/orion-tts.sock
```

Terminal 2 owns Orion hardware and playback:

```bash
sudo runtime/target/release/oriond --serve \
  --backend hardware \
  --port /dev/ttyACM0 \
  --baud-rate 1000000 \
  --calibration /home/mofe/.config/orion/servo_calibration.json
```

Terminal 3 submits speech:

```bash
runtime/target/release/oriond \
  --speak "Hello. I am Orion." \
  --wait
```

The speech lifecycle remains:

```text
synthesizing -> playing -> completed
             \-> failed
synthesizing/playing -> cancelled
```

Inspect or cancel it with:

```bash
runtime/target/release/oriond --speech-status
runtime/target/release/oriond --stop-speech
```

Generated WAV files are removed after playback or cancellation. They are not a
speech archive.

## Run wake-word detection

Terminal 1 starts local microphone capture and inference:

```bash
cd /home/mofe/dev/orion
voice/.venv/bin/orion-voice wake-worker
```

The worker first applies Orion's ReSpeaker V2 capture contract: the two
single-ended microphone routes, fixed 40 dB PGA gain, and codec AGC disabled.
It then opens the stable ALSA capture device at 16 kHz mono.

Wait for:

```text
orion-wake: listening for HEY ORION
```

In terminal 2, subscribe before speaking:

```bash
cd /home/mofe/dev/orion
voice/.venv/bin/orion-voice wait-wake
```

Say **“Hey Orion.”** A successful detection returns:

```json
{"event_id":1,"event":"wake_word","phrase":"HEY ORION"}
```

The worker continues listening and increments `event_id`. A subscriber receives
future events only; the socket is an event stream, not a history database.

The defaults are a `0.25` trigger threshold, `1.5` keyword score, and two CPU
threads. If physical testing misses clear utterances, lower the threshold in a
small step:

```bash
voice/.venv/bin/orion-voice wake-worker --threshold 0.20
```

A lower threshold is easier to trigger and can increase false positives. Keep
the default unless repeated physical trials justify changing it.

## Current voice-loop boundary

The wake worker and future speech recognizer must not open independent capture
streams indefinitely. The STT slice will create one microphone owner that
switches between:

```text
wake listening -> command capture/transcription -> wake listening
```

For now, the wake event proves activation only. It does not capture the words
after `HEY ORION`, route an intent, move Orion, or synthesize a response.

## Tests

The unit suite uses fake Piper chunks, fake Sherpa streams, and temporary Unix
sockets, so it does not need downloaded models or robot hardware:

```bash
cd /home/mofe/dev/orion
PYTHONPATH=voice python3 -m unittest discover -s voice/tests -v
```

The tests cover request validation, 48 kHz stereo speech conversion, ephemeral
WAV cleanup, selected-voice identity, wake-model validation, streaming keyword
decoding, ALSA capture arguments, event ordering, and stale-socket cleanup.
