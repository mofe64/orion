# Orion local audio cues

Short, deterministic sounds played by Orion scenes live in this directory.
Hardware routing and ReSpeaker commissioning remain under `hardware/audio/`;
the cue files are portable scene resources.

Each cue is addressed by its filename stem. For example, a scene action with
`cue: acknowledge_warm` resolves to `audio/cues/acknowledge_warm.wav`. Names
may contain ASCII letters, digits, hyphens, and underscores.

Tracked cues use uncompressed RIFF/WAVE audio. Orion's first physical speaker
is the mono JST output on the ReSpeaker V2 HAT, fed from the right playback
channel. Cue assets therefore contain identical left and right channels so
they also preview naturally on normal stereo equipment.

`generate_cues.py` creates Orion's warm tonal vocabulary using only Python's
standard library. It is an authoring tool, not a dependency of `oriond`:

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
