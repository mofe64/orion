# 0004: Retain Pi-local voice as a fallback

- **Status:** Accepted
- **Scope:** Raspberry Pi wake, transcription, and generated speech

## Context

Studio Voice provides the higher-quality experience, but hardware diagnosis
and offline operation still benefit from a self-contained Pi path. Removing it
would also remove a commissioned ReSpeaker capture and playback test surface.

## Decision

Retain the Sherpa/Silero/Moonshine listener and Piper worker under `voice/` as a
diagnostic and offline fallback. The listener publishes transcript events but
does not interpret them. Piper returns temporary speech audio to `oriond`,
which remains the owner of ReSpeaker playback and speech lifecycle.

## Consequences

- The robot can validate local capture, wake, speech recognition, generation, and playback
  without Studio.
- Pi model installation and Studio model installation remain separate.
- The fallback does not become a second agent or hardware-control authority.
- The Pi processes must remain separate from the primary Studio Voice path.
