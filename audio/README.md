# Orion local audio cues

This directory is the source of truth for short, deterministic sounds played
by Orion scenes. Hardware routing and ReSpeaker commissioning remain under
`hardware/audio/`; these files are portable scene resources.

Each cue is addressed by its filename stem. For example, a scene action with
`cue: acknowledge` resolves to `audio/cues/acknowledge.wav`. Names may contain
ASCII letters, digits, hyphens, and underscores.

Tracked cues use uncompressed RIFF/WAVE audio. Orion's first physical speaker
is the mono JST output on the ReSpeaker V2 HAT, fed from the right playback
channel. Cue assets therefore contain identical left and right channels so
they also preview naturally on normal stereo equipment.

`generate_cues.py` creates Orion's original cue sounds using only Python's
standard library. It is an authoring tool, not a dependency of `oriond`:

```bash
python3 audio/generate_cues.py
```

The first cue, `acknowledge`, is a restrained 420 ms ascending two-note sound.
It is used by the left and right expressive acknowledgement scenes.
