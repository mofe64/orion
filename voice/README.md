# Orion local voice worker

`voice/` contains Orion's local machine-learning voice processes. The current
slice is a persistent Piper text-to-speech worker. It loads one ONNX voice,
accepts JSON-line requests over `/tmp/orion-tts.sock`, and writes temporary
48 kHz, 16-bit stereo WAV files.

The worker does not open ALSA and does not control motion or lighting. The Rust
daemon receives the generated path and plays it through its existing
`AudioDevice`, so cues, generated speech, scene completion, and the ReSpeaker
JST route keep one physical audio owner. Speech IDs, status, cancellation, and
`--wait` remain independent of the selected TTS model.

## Python environment

Orion currently keeps voice dependencies in a user-owned Python 3.11
environment, separate from Debian and the repository-level MuJoCo environment:

```bash
cd /home/mofe/dev/orion

curl -LsSf https://astral.sh/uv/install.sh | sh
/home/mofe/.local/bin/uv python install 3.11
/home/mofe/.local/bin/uv sync --project voice --python 3.11

voice/.venv/bin/python --version
```

The final command must report Python `3.11.x`. Do not use `sudo` for these
commands and do not replace `/usr/bin/python3`.

`uv sync` makes `voice/.venv` match `voice/pyproject.toml`. During the
Chatterbox-to-Piper migration it removes Chatterbox, PyTorch, Perth,
Praat-Parselmouth, and the other packages that Piper does not require.

## Download the benchmark voice

Voice models are runtime data and are ignored by Git. Download the initial
63 MB `en_US-lessac-medium` candidate into `voice/models/`:

```bash
voice/.venv/bin/python -m piper.download_voices \
  --download-dir voice/models \
  en_US-lessac-medium
```

This creates both files required by Piper:

```text
voice/models/en_US-lessac-medium.onnx
voice/models/en_US-lessac-medium.onnx.json
```

The worker reports a clear error with this download command if either file is
missing.

## Benchmark Piper first

Run the same three-iteration benchmark used for Chatterbox:

```bash
voice/.venv/bin/orion-voice benchmark-tts \
  --text "Hello. I am Orion, and my voice is running locally." \
  --iterations 3
```

The command prints JSON containing model-load time, synthesis time, generated
audio duration, realtime factor, and peak resident memory. A realtime factor
below `1.0` means synthesis completed faster than the produced speech.
Benchmark WAVs are written to `/tmp/orion-tts-benchmark/`.

Listen through Orion's commissioned ReSpeaker route:

```bash
hardware/audio/configure-playback.sh
aplay -D plughw:CARD=seeed2micvoicec,DEV=0 \
  /tmp/orion-tts-benchmark/piper-2.wav
```

Piper produces sentence chunks internally. This adapter currently collects
them into one temporary WAV because `oriond` owns playback by file. The chunked
API leaves a later path to playback-before-completion without changing models.

## Run from source

Terminal 1 owns the model:

```bash
cd /home/mofe/dev/orion
voice/.venv/bin/orion-voice tts-worker
```

Wait for:

```text
orion-tts: ready on /tmp/orion-tts.sock
```

Terminal 2 owns Orion hardware as before:

```bash
sudo runtime/target/release/oriond --serve \
  --backend hardware \
  --port /dev/ttyACM0 \
  --baud-rate 1000000 \
  --calibration /home/mofe/.config/orion/servo_calibration.json
```

Terminal 3 submits speech:

```bash
runtime/target/release/oriond --speak "Hello. I am Orion." --wait
```

The worker and daemon remain foreground source processes; this slice does not
install either as a boot service.

## Lifecycle and ownership

A speech run follows:

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

The daemon retains only the active speech run and most recent terminal result.
Run IDs reset on daemon restart. Generated WAV files are removed after playback
or cancellation; they are not a speech archive. `--wait` exits `0` for
completed, `5` for cancelled, and `7` for failed.

Standalone speech and authored scenes are mutually exclusive in this first
slice because they share the physical audio backend. Parameterized speech
inside a coordinated motion/light scene is a later scene-contract change.

## Try another Piper voice

Download another voice, then pass its model explicitly to both benchmarking and
the worker:

```bash
voice/.venv/bin/python -m piper.download_voices \
  --download-dir voice/models \
  VOICE_NAME

voice/.venv/bin/orion-voice benchmark-tts \
  --model voice/models/VOICE_NAME.onnx

voice/.venv/bin/orion-voice tts-worker \
  --model voice/models/VOICE_NAME.onnx
```

This lets us choose Orion's permanent voice by listening on the physical robot
without changing source code.

## Remove the obsolete Chatterbox data

First complete the Piper benchmark and listening check. Then inspect and remove
the old downloadable Chatterbox checkpoint:

```bash
du -sh /home/mofe/.cache/huggingface/hub/models--ResembleAI--chatterbox-nano
rm -rf -- /home/mofe/.cache/huggingface/hub/models--ResembleAI--chatterbox-nano
```

The deletion is limited to the reproducible Chatterbox Nano cache; it does not
touch other Hugging Face models. It can be recovered by downloading Chatterbox
again. Finally, discard unused uv cache entries:

```bash
/home/mofe/.local/bin/uv cache prune
```

## Tests without downloading the model

The adapter and worker tests inject fake Piper voices:

```bash
cd /home/mofe/dev/orion
PYTHONPATH=voice python3 -m unittest discover -s voice/tests -v
```

These tests verify request validation, model/config validation, 48 kHz stereo
conversion, Unix-socket transport, WAV-result reporting, and stale-socket
cleanup without importing Piper or downloading a voice.
