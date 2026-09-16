# Orion local audio cues

Short, deterministic sounds played by Orion scenes live in this directory.
The [audio hardware guide](../hardware/audio/README.md) covers routing and
ReSpeaker setup. Cue files are portable scene resources.

Each cue is addressed by its filename stem. For example, a scene action with
`cue: acknowledge_warm` resolves to `audio/cues/acknowledge_warm.wav`. Names
may contain ASCII letters, digits, hyphens, and underscores.

Tracked cues use uncompressed RIFF/WAVE audio. Orion's first physical speaker
is the mono JST output on the ReSpeaker V2 HAT, fed from the right playback
channel. Cue assets therefore contain identical left and right channels so
they also preview naturally on normal stereo equipment.

`generate_cues.py` creates Orion's warm tonal vocabulary using only Python's
standard library. Use it when authoring or retuning cues:

```bash
python3 audio/generate_cues.py
```

The v2 vocabulary is `notice_warm`, `acknowledge_warm`, `curious_rise`,
`agree_soft`, `delight_warm`, `settle_soft`, and `error_muted`. It uses one
coherent harmonic palette, gentle attacks, short decays, and restrained
loudness. Routine idles never dispatch a cue.

## Voice entry cues

`voice_wake` is a 200 ms, 220 Hz rounded tone. `voice_processing` is a 180 ms,
180 Hz tone. `VOICE_CUE_GAIN` in the generator sets their levels independently
of reply speech and the global ALSA mixer. Regenerate with
`python3 audio/generate_cues.py` from the repository root after tuning. The
runtime plays each entry cue under the voice session guard; speech preempts it.
Physical loudness, microphone pickup and endpoint effects require Pi validation.

## Alarm and timer sounds

The runtime offers the generated **Two-tone** sound by default, plus **Club alarm**
and **Funny alarm** from the MP3s in `alarms/`. Studio **Settings → Voice and sounds**
saves separate choices for alarms and timers on Orion. See
[timer and alarm behavior](../docs/system-architecture.md#timers-and-alarms) for
repetition, dismissal and overlapping alerts.

The original MP3 filenames retain their source identifiers. `prepare_alarms.py`
uses ffmpeg to produce `club_alarm.pcm` and `funny_alarm.pcm`: 24 kHz mono, signed
16-bit little-endian audio, with a 0.65 peak and 12 ms fades at both ends. After
editing a source recording, regenerate and commit its PCM file:

```bash
python3 audio/prepare_alarms.py
```

Both PCM files are embedded in `oriond` at build time. Installation and playback
require no ffmpeg or MP3 decoder, and the sounds travel with the code release
independently of the Pi's scene catalog. This adds about 1.2 MB of audio data to
the runtime binary. Rebuild `oriond` after regenerating these files.
