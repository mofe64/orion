# Orion local voice worker

`voice/` contains Orion's local machine-learning voice processes. The first
slice is a persistent Chatterbox Nano text-to-speech worker. It loads the model
once, uses Nano's built-in voice, accepts JSON-line requests over
`/tmp/orion-tts.sock`, and writes temporary 48 kHz, 16-bit stereo WAV files.

The worker does not open ALSA and does not control motion or lighting. The Rust
daemon receives the generated path and plays it through its existing
`AudioDevice`, so cues, generated speech, scene completion, and the ReSpeaker
JST route keep one physical audio owner.

## Python environment

Chatterbox documents Python 3.11 as its tested environment. Debian 13 ships a
newer system Python, so use `uv` to install a user-owned CPython 3.11 and keep
it separate from both Debian and the repository-level MuJoCo environment:

```bash
cd /home/mofe/dev/orion

curl -LsSf https://astral.sh/uv/install.sh | sh
/home/mofe/.local/bin/uv python install 3.11
/home/mofe/.local/bin/uv sync --project voice --python 3.11

voice/.venv/bin/python --version
```

The final command must report Python `3.11.x`. Do not use `sudo` for these
commands and do not replace `/usr/bin/python3`; Orion's voice environment is
entirely local to `voice/.venv`.

Orion pins Chatterbox to the official source revision that introduced Nano.
The current PyPI release contains the older Turbo-only loader even though its
package version matches the source project. After pulling a dependency change,
refresh the voice environment with:

```bash
/home/mofe/.local/bin/uv sync --project voice --python 3.11 \
  --refresh-package chatterbox-tts
```

The first model load downloads the `ResembleAI/chatterbox-nano` checkpoint and
therefore needs internet access. Subsequent source-run sessions use the local
model cache.

## Benchmark Nano first

Run the benchmark before treating Nano as Orion's selected physical TTS:

```bash
voice/.venv/bin/orion-voice benchmark-tts \
  --text "Hello. I am Orion, and my voice is running locally." \
  --iterations 3
```

The command prints JSON containing model-load time, synthesis time, generated
audio duration, realtime factor, and peak resident memory. A realtime factor
below `1.0` means synthesis completed faster than the duration of the produced
speech. Benchmark output WAVs are written to `/tmp/orion-tts-benchmark/` for
listening and are not part of Orion's cue library.

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

## Tests without downloading the model

The protocol and worker tests inject a fake synthesizer:

```bash
cd /home/mofe/dev/orion
PYTHONPATH=voice python3 -m unittest discover -s voice/tests -v
```

These tests verify request validation, Unix-socket transport, WAV-result
reporting, and stale-socket cleanup without importing PyTorch or Chatterbox.
