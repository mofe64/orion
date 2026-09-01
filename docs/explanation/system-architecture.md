# Orion system architecture

## System boundary

Orion is split between a workstation and a Raspberry Pi:

```text
Workstation                                              Raspberry Pi

┌──────────────────────────────┐       HTTP v1       ┌─────────────────────┐
│ Orion Studio                 │ ──────────────────▶ │ Studio gateway      │
│                              │   bearer token      │                     │
│ • author and preview assets  │                     │ • authenticate      │
│ • submit semantic commands   │                     │ • validate API      │
│ • run primary voice pipeline │                     │ • adapt requests    │
└──────────────────────────────┘                     └──────────┬──────────┘
                                                               │ private
                                                               │ Unix socket
                                                               ▼
                                                    ┌─────────────────────┐
                                                    │ oriond              │
                                                    │                     │
                                                    │ • lifecycle/safety  │
                                                    │ • asset validation  │
                                                    │ • trajectory timing │
                                                    │ • scene coordination│
                                                    │ • device ownership  │
                                                    └──────────┬──────────┘
                                                               │
                                        ┌──────────────────────┼───────────┐
                                        ▼                      ▼           ▼
                                     servos                  RGBW        audio
```

The same `oriond` state machine can use the physical hardware backend or the
MuJoCo backend. Simulation is therefore a different device implementation,
not a separate motion system.

## Authority boundaries

### `oriond`

`oriond` is the sole Raspberry Pi hardware authority. It owns the serial bus,
RGBW output device, and ReSpeaker playback device while the hardware daemon is
running. It also owns lifecycle transitions, calibration conversion, safety
limits, interpolation, run IDs, completion checks, and cancellation.

No Studio or agent operation may bypass this boundary. Higher-level systems
request named capabilities such as `home`, `look_at_left_expressive`, or
`acknowledge_left`; they do not submit register writes or arbitrary joint
streams.

### Studio gateway

The gateway is a deliberately narrow network adapter. It authenticates Studio
on a trusted development LAN, exposes versioned semantic operations, and keeps
`/tmp/oriond.sock` private to the Pi. It rejects arbitrary filesystem paths,
servo registers, and raw device commands.

### Orion Studio

Studio owns desktop interaction: asset browsing, editing, local preview,
connection state, and the optional workstation voice experience. Editing a
slider or timeline never moves hardware. Physical execution requires an
explicit connected run request through the gateway.

Studio may open the workstation microphone and speaker after the user enables
Voice. It does not open the Pi's servo, lighting, or audio devices.

## Asset flow

Built-in poses, motions, and scenes are immutable source material. User assets
live in dedicated directories:

```text
motion/user/poses/
motion/motions/user/
scenes/user/
```

When offline, Studio writes to its desktop checkout as a staging area. When
connected, the Pi library is authoritative. A Pi asset wins over an offline
asset with the same user-defined name, and no user asset may shadow a built-in.

Creating or updating a Pi asset is transactional from Studio's perspective:
the gateway writes it, asks `oriond` to reload and validate the complete
catalog, and rolls back the file if validation fails. Scene updates include
the revision Studio loaded so a stale editor cannot silently overwrite a newer
version.

## Runtime state

Movement follows this lifecycle:

```text
executing -> settling -> completed
                      \-> timed_out
executing/settling ----> cancelled
```

Scenes coordinate movement, lighting, and audio under one monotonic clock.
They finish only when all dispatched work has reached a terminal result. The
daemon retains the active run and most recent terminal result; it is not a
durable event database, and run IDs reset after restart.

The daemon starts after reboot in torque-off observe mode. The first explicit
movement request prepares and enables the servo path. Lighting- or audio-only
work does not energize the servos.

## Important invariants

- `oriond` is the only owner of Pi hardware devices.
- Every physical request crosses a semantic, validated capability boundary.
- Calibration and runtime limits are applied before movement reaches a driver.
- Studio editing is inert until the user explicitly requests hardware preview
  or execution.
- Built-in assets cannot be overwritten or shadowed.
- User asset updates use revision checks and catalog-wide validation.
- Raw microphone audio remains transient; see the
  [voice architecture](voice-architecture.md) for the point at which text may
  leave the workstation.

## Related components

- [Rust runtime](../../runtime/README.md)
- [Orion Studio](../../orion_studio/README.md)
- [Motion assets](../../motion/README.md)
- [Scene format](../../scenes/README.md)
- [Robot description](../../description/README.md)
- [MuJoCo model](../../simulation/mujoco/README.md)
- [Voice architecture](voice-architecture.md)
