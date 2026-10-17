# Orion audio assets

`audio/cues/` contains short WAV sounds for scenes and voice feedback. A cue name is its file name without `.wav`. Names contain ASCII letters, digits, hyphens or underscores.

Cue files contain matching left and right channels. The runtime selects playback routing from the hardware profile. Routine idle motion does not play cues.

## Generate cues

Run these steps from the repository root. The generator uses Python's standard library.

1. Regenerate the cue WAV files.

   ```bash
   uv run --project motion --locked --managed-python python audio/generate_cues.py
   ```

2. Rebuild the runtime after changing embedded alarm audio.

The cue vocabulary includes `notice_warm`, `acknowledge_warm`, `curious_rise`, `agree_soft`, `delight_warm`, `settle_soft` and `error_muted`. `voice_wake` and `voice_processing` provide voice-entry feedback. `VOICE_CUE_GAIN` controls their generator level.

## Alarm audio

`audio/alarms/` contains MP3 sources and embedded PCM files. The runtime offers Two-tone, Club alarm and Funny alarm. Studio stores separate alarm and timer choices.

The PCM format is mono, 24 kHz, signed 16-bit little-endian audio. Playback does not require ffmpeg. Regeneration requires ffmpeg.

1. Convert changed alarm sources into runtime PCM files.

   ```bash
   uv run --project motion --locked --managed-python python audio/prepare_alarms.py
   ```

2. Rebuild `oriond`.

See [alert behavior](../docs/system-architecture.md#timers-and-alarms) and [V1 audio hardware](../hardware/audio/README.md).
