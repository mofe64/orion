# 0002: Expose semantic capabilities, not raw hardware

- **Status:** Accepted
- **Scope:** Studio, agents, gateway, and physical devices

## Context

Desktop tools and intelligent agents need to request useful behaviour without
gaining enough authority to bypass calibration, limits, lifecycle, or device
ownership. Raw joint streams, servo registers, GPIO writes, and arbitrary file
paths would make that guarantee impossible to audit.

## Decision

Keep the serial bus, RGBW output, and Pi playback devices under `oriond`.
Expose allow-listed named poses, motions, scenes, speech, status, cancellation,
and revision-checked asset operations through the Studio gateway.

Any future agent-to-robot integration must end in the same validated semantic
operations. Agent text or model output is never itself a hardware command.

## Consequences

- Studio editing can remain inert until an explicit Run operation.
- Hardware safety and completion semantics have one implementation.
- The network surface is smaller and rejects low-level device access.
- Adding a new capability requires an explicit runtime contract and validation
  rather than an unreviewed prompt or adapter change.

