# Orion Raspberry Pi-local fallback voice runtime

`voice/` contains Orion's Raspberry Pi-local fallback and diagnostic speech
processes. Studio Voice is the primary interactive path; see the
[voice architecture](../docs/explanation/voice-architecture.md). The Pi path
has two narrow runtime boundaries:

```text
agent/oriond -> /tmp/orion-tts.sock -> Piper Ryan Medium -> temporary WAV
microphones -> Sherpa wake/VAD/Moonshine -> /tmp/orion-wake.sock -> voice events
```

Piper owns text-to-speech model inference. `oriond` remains the only owner of
physical playback, so generated speech keeps the same speech IDs, status,
`--wait`, cancellation, and ALSA lifecycle as cues and scenes.

The listener owns transient 16 kHz mono microphone capture, detects the
phrase `HELLO WORLD`, segments the following command with Silero VAD, and
transcribes it locally with Moonshine Tiny English INT8. It publishes ordered
JSON-line events and never writes microphone audio to disk. It does not
interpret the resulting transcript or invoke an agent.

## Install the Python environment

Orion uses a Python 3.11 environment because the selected Pi voice packages do
not support the Pi's newer system Python:

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

It performs six bounded operations:

1. Downloads `en_US-ryan-medium`, Orion's selected production voice.
2. Downloads Sherpa's English 3.3-million-parameter GigaSpeech keyword model.
3. Generates the model-specific BPE tokens for `HELLO WORLD`.
4. Downloads the quantized Moonshine Tiny English speech-recognition model.
5. Downloads the Silero VAD model used to delimit commands.
6. Removes other top-level Piper voice files while retaining nested voice models.

The resulting runtime data is ignored by Git:

```text
voice/models/en_US-ryan-medium.onnx
voice/models/en_US-ryan-medium.onnx.json
voice/models/wake/sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01/
voice/models/asr/sherpa-onnx-moonshine-tiny-en-int8/
voice/models/vad/silero_vad.onnx
```

To repeat only the selected-voice cleanup:

```bash
voice/cleanup-voices.sh
```

The cleanup refuses to run unless both Ryan Medium files are already present.
It removes only `.onnx` and `.onnx.json` files directly inside `voice/models/`;
it does not touch nested wake-word or speech-recognition models.

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
single-ended microphone routes, fixed 50 dB PGA gain, and codec AGC disabled.
It then opens the stable ALSA capture device at 16 kHz mono.

Wait for:

```text
orion-wake: listening for HELLO WORLD
```

In terminal 2, subscribe before speaking:

```bash
cd /home/mofe/dev/orion
voice/.venv/bin/orion-voice wait-wake
```

Say **“Hello world.”** A successful detection returns:

```json
{"event_id":1,"event":"wake_word","phrase":"HELLO WORLD"}
```

The worker continues listening and increments `event_id`. A subscriber receives
future events only; the socket is an event stream, not a history database.

To commission a different phrase without reinstalling the model, stop the wake
worker and regenerate its small BPE keyword file:

```bash
voice/configure-wake-word.sh "HEY ORION"
```

Restart the worker after changing the phrase; it reads the keyword file when it
starts. `HELLO WORLD` is Orion's selected phrase because it is used in Sherpa's
English GigaSpeech keyword-customization documentation and passed Orion's
physical microphone test. Restore the selected phrase with:

```bash
voice/configure-wake-word.sh "HELLO WORLD"
```

The physically commissioned defaults are a `0.10` trigger threshold, `3.0`
keyword score, and two CPU threads. These can still be overridden for controlled
experiments:

```bash
voice/.venv/bin/orion-voice wake-worker --score 2.5 --threshold 0.15
```

A lower threshold or higher score is easier to trigger and can increase false
positives. Keep the commissioned defaults unless repeated physical trials
justify changing them.

## Run wake and speech-to-text together

Do not run `wake-worker` at the same time. The normal listener is the single
microphone owner and switches between:

```text
wake listening -> command capture -> transcription -> wake listening
```

Terminal 1 loads the wake, VAD, and ASR models once:

```bash
cd /home/mofe/dev/orion
voice/.venv/bin/orion-voice listen-worker
```

Wait for:

```text
orion-listener: waiting for HELLO WORLD
```

In terminal 2, subscribe before speaking:

```bash
cd /home/mofe/dev/orion
voice/.venv/bin/orion-voice wait-command
```

Say **“Hello world”**, pause briefly, then say a command such as **“Return
home.”** The event stream contains the activation followed by a terminal command
result; `wait-command` prints the result:

```json
{"event_id":2,"event":"command","state":"transcribed","text":"Return home.","audio_seconds":1.25,"inference_seconds":0.2,"error":null}
```

The command states and CLI exit codes are:

| State | Meaning | Exit code |
| --- | --- | ---: |
| `transcribed` | Moonshine produced non-empty text | `0` |
| `timed_out` | No complete utterance arrived within 8 seconds | `2` |
| `failed` | VAD/ASR processing failed or returned empty text | `1` |

Silero requires 0.8 seconds of trailing silence to close the command and limits
one speech segment to 10 seconds. Completed samples are released after
transcription; no WAV or transcript history is stored by the worker.

This slice stops at transcript publication. It does not yet interpret the text,
invoke Orion capabilities, move Orion, or synthesize a response.

## Tests

The unit suite uses fake Piper chunks, fake Sherpa streams, and temporary Unix
sockets, so it does not need downloaded models or robot hardware:

```bash
cd /home/mofe/dev/orion
PYTHONPATH=voice python3 -m unittest discover -s voice/tests -v
```

The tests cover request validation, 48 kHz stereo speech conversion, ephemeral
WAV cleanup, selected-voice identity, wake-model validation, streaming keyword
decoding, Silero segmentation, Moonshine transcription, fake-clock command
timeouts, ALSA capture arguments, ordered voice events, and stale-socket cleanup.
