# Rust voice coordinator and inference worker

Status: implemented locally; physical voice acceptance remains outstanding.

## Context

The Python worker owned Pi transport, transcription, agent calls, buffering,
uploads, and playback. Moving the agent into Rust left Python coordinating it
through a local text client. That coupled application flow to the inference
process and made a future launcher depend on Studio's worker layout.

## Decision

Use a top-level Rust `orion-coordinator` library to own the voice pipeline.
Call `orion-agent` through an in-process handle. Keep Python model execution in
`speech/`, controlled by identified jobs over private pipes. Studio supplies
settings, pairing, startup/shutdown, and UI observers.

Preserve Pi protocol 1 and Studio observer protocol 7. Hardware operations still
use the authenticated gateway and `oriond`. The agent service has an independent
lifetime so idle coordinator/model reloads retain conversational context.

## Consequences

The Python agent client and processing server are removed. Shared orchestration
can be reused by a future headless launcher without depending on Tauri.
Cancelling active native inference retires the speech child and reloads models
for the next job. Successful jobs reuse the loaded models. Failed synthesis must
never be confused with successful stream completion; an explicit end marker is
required before releasing a complete-reply startup buffer.
