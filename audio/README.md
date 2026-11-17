# Orion audio assets

`audio/cues/` contains short WAV sounds for scenes and voice feedback. A cue name is its file name without `.wav`. Names contain ASCII letters, digits, hyphens or underscores.

Cue files contain matching left and right channels. The runtime selects playback routing from the hardware profile. Routine idle motion does not play cues.

## Volume

Orion applies one master volume to speech, cues, alarms and music. The range is 0–100.
The runtime saves this setting in `~/.config/orion/audio.json` and loads it at startup.
Alarms use at least 25 percent volume, including when Orion is muted.
The XVF3800 hardware PCM controls stay at unity gain. The runtime scales each source before playback.

Use the Volume slider on the Orion page. Changes apply while you move the slider.
Say “Volume 40”, “Louder” or “Quieter” to change it by voice.
Louder and Quieter change the level by 10.

## Build cues

The cues are short voice-like sounds. Each cue is one part of a recording from the Reachy Mini Emotions Library with a lower pitch.
See [third-party notices](../THIRD_PARTY_NOTICES.md#reachy-mini-emotions-library) for the source and license.

Run these steps from the repository root. The builder needs a local copy of the library.

1. Download the library into a directory.

   ```bash
   uvx --from huggingface_hub hf download pollen-robotics/reachy-mini-emotions-library --repo-type dataset --local-dir reachy-emotions
   ```

2. Rebuild the cue WAV files.

   ```bash
   uv run --with librosa --with soundfile python audio/build_cues.py reachy-emotions
   ```

[The cue builder](build_cues.py) contains the source recording, the cut times and the peak level for each cue.

| Cue | Orion plays it when |
|---|---|
| `voice_wake` | Orion hears its wake word |
| `voice_processing` | A command ends and Orion starts to think |
| `didnt_catch` | Orion hears its wake word but no command |
| `error_muted` | A voice request fails or the voice service is not available |
| `wake_chirp`, `sleep_sigh`, `yawn_hum` | The wake stretch, sleep settle or yawn reaches its marker |
| `think_hum` | A thinking glance reaches its marker |
| `notice_warm` | Orion notices an arrival or starts an announcement |
| `curious_rise` | Orion peeks at a new object |
| `delight_warm` | Orion waves back |
| `acknowledge_warm`, `settle_soft` | A scene requests the cue |

`listen_hm`, `agree_soft`, `disagree_soft`, `success_bright`, `laugh_light`, `surprise_pop` and `sorry_low` are available to scenes.
`act_sounds` controls the sounds that follow motion markers.
See [character behaviors](../docs/configuration.md#character-behaviors) for switches and [sound ownership](../docs/motion-architecture.md#shared-mind-state) for suppression rules.

## Alarm audio

`audio/alarms/` contains MP3 sources and embedded PCM files. The runtime offers Soft chime, Two-tone, Club alarm and Funny alarm.
Soft chime is the default. It repeats three notes with a listening pause and increases volume over 30 seconds.
Studio stores separate alarm and timer choices.

The PCM format is mono, 24 kHz, signed 16-bit little-endian audio. Playback does not require ffmpeg. Regeneration requires ffmpeg.

1. Convert changed alarm sources into runtime PCM files.

   ```bash
   uv run --project motion --locked --managed-python python audio/prepare_alarms.py
   ```

2. Rebuild `oriond`.

See [alert behavior](../docs/system-architecture.md#timers-and-alarms) and [V1 audio hardware](../hardware/audio/README.md).
