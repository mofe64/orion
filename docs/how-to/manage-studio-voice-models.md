# Manage Studio Voice models

Studio Voice uses Hugging Face model repositories. Git contains adapter code
and a 104 KB Rustpotter reference, but not the automatic speech recognition
(ASR), text-to-speech (TTS), or tokenizer weights.

## Download the defaults

```bash
cd orion_studio/voice_worker
uv sync --python 3.12
.venv/bin/orion-voice-models
```

The downloader calls `huggingface_hub.snapshot_download` for the model IDs
defined in `orion_voice_worker/models.py`. Existing verified cache content is
reused, so rerunning it is safe and normally downloads only missing files.

The default cache is `~/.cache/huggingface/hub`. Set `HF_HOME` before both the
download command and Studio if the cache must live elsewhere:

```bash
HF_HOME=/absolute/path/to/hugging-face-cache \
  .venv/bin/orion-voice-models

cd ..
HF_HOME=/absolute/path/to/hugging-face-cache \
  pnpm tauri dev
```

Do not place downloaded weights under `voice_worker/models/`; that directory
contains the small commissioned Rustpotter reference and remains tracked.

## Use a local or alternative compatible model

Set overrides before starting the Tauri process:

```bash
cd orion_studio
ORION_STUDIO_ASR_MODEL=/absolute/path/to/qwen3-asr-model \
ORION_STUDIO_TTS_MODEL=/absolute/path/to/chatterbox-model \
pnpm tauri dev
```

The values may be compatible Hugging Face repository IDs or local paths
accepted by the corresponding adapter. Changing an ID does not guarantee
compatibility; the model must match the APIs expected by the Qwen or
Chatterbox implementation.

## Prepare a device for offline use

1. Connect the device to the network.
2. Run `.venv/bin/orion-voice-models` to completion.
3. Start Studio and complete one voice response while still online.
4. Confirm that all models load from the cache before disconnecting.

Copying a cache from another machine can work, but the destination must use a
compatible processor, Python environment, adapter versions, and complete
Hugging Face snapshot structure. Running the downloader on the target device
is the supported development path.

## Remove cached weights

The downloader does not own the entire Hugging Face cache, which may contain
models used by other applications. Inspect the cache and remove only the
specific Qwen, Chatterbox, and tokenizer snapshots you intend to discard.
Avoid deleting the whole cache as part of an Orion script.
