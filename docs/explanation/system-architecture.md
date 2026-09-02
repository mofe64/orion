# Orion system architecture

## System boundary

Orion is split between a workstation and a Raspberry Pi:

RGBW means the red, green, blue, and dedicated white channels in Orion's light.

```text
Workstation                                              Raspberry Pi

┌──────────────────────────────┐       HTTP v2       ┌─────────────────────┐
│ Orion Studio                 │ ──────────────────▶ │ Studio gateway      │
│                              │   bearer token      │                     │
│ • author and preview assets  │                     │ • authenticate      │
│ • submit semantic commands   │                     │ • validate API      │
│ • synthesize voice responses │                     │ • spool speech WAV  │
└──────────────────────────────┘                     └──────────┬──────────┘
                                                               │ private
                                                               │ Unix socket
                                                               ▼
                                                    ┌─────────────────────┐
                                                    │ oriond              │
                                                    │                     │
                                                    │ • lifecycle/safety  │
                                                    │ • asset validation  │
                                                    │ • Rust spline engine│
                                                    │ • character priority│
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
40-pixel RGBW output, and ReSpeaker playback while running. It also owns the
explicit character state machine, live-calibration conversion, continuous
trajectory compilation, semantic run IDs, completion checks, and cancellation.

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

Studio may open the workstation microphone after the user enables Voice and
synthesizes Chatterbox PCM locally. The authenticated gateway transfers the
validated WAV to the Pi; primary playback is Orion's ReSpeaker, not the
workstation speaker. Studio never opens Pi devices directly.

## Asset flow

Built-in poses, motions, and scenes are immutable source material. User assets
live in dedicated directories:

```text
motion/user/poses/
motion/motions/user/
scenes/user/
```

Studio's source catalog is available offline for browsing and editing. Publish
requires a connected Pi, whose user library and live calibration are
authoritative. No user asset may shadow a built-in.

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

Character mode is a separate explicit state and also starts disabled after
every restart. Starting it configures torque, moves to powered `home`, captures
an immutable idle anchor, and schedules randomized relative idles. Foreground
work outranks speech, reactions, and idle. Mechanical `rest` is reserved for
shutdown before torque release.

## Important invariants

- `oriond` is the only owner of Pi hardware devices.
- Every physical request crosses a semantic, validated capability boundary.
- Pi calibration is the only position-limit authority; the 7.4 V STS3215
  profile supplies the 52 RPM capability ceiling.
- Hardware and MuJoCo consume the same Rust-compiled 50 Hz trajectory.
- Studio editing is inert until the user explicitly requests hardware preview
  or execution.
- Built-in assets cannot be overwritten or shadowed.
- User asset updates use revision checks and catalog-wide validation.
- Raw microphone audio remains transient; see the
  [voice architecture](voice-architecture.md) for the point at which text may
  leave the workstation.

## Related components

- [Motion and animation architecture](motion-and-animation-architecture.md)
- [Character animation design](character-animation.md)
- [Trajectory and joint-control reference](../reference/trajectory-and-joint-control.md)
- [Motion asset reference](../reference/motion-assets.md)
- [Rust runtime](../../runtime/README.md)
- [Orion Studio](../../orion_studio/README.md)
- [Motion assets](../../motion/README.md)
- [Scene format](../../scenes/README.md)
- [Robot description](../../description/README.md)
- [MuJoCo model](../../simulation/mujoco/README.md)
- [Voice architecture](voice-architecture.md)
