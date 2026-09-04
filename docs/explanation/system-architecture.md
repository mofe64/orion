# Orion system architecture

## System boundary

Orion is split between an external computer and an onboard computer (raspberry Pi):


```text
External Computer                                     Onboard Computer

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
MuJoCo backend. So the same motion system drives physical servos and simulation

## Authority boundaries

### `oriond`

`oriond` is the sole onboard hardware authority. It owns the serial bus,
Lighting RGBW output, and ReSpeaker playback while running. It also owns the
explicit character state machine, live-calibration conversion, continuous
trajectory compilation, semantic run IDs, completion checks, and cancellation.

No Studio or agent operation may bypass this boundary. Higher-level systems
request named capabilities; but they do not submit servo register writes or arbitrary joint
streams.

### Studio gateway

The gateway is a deliberately narrow network adapter. It authenticates requests from
orion studio, exposes versioned semantic operations, and keeps
`/tmp/oriond.sock` private to the onboard computer.

### Orion Studio

Studio owns desktop interaction: asset browsing, editing, local preview,
connection state, and the optional workstation voice experience.
Note - the voice experience is going to be revamped to do user voice capture from the robots
embedded mics
Orion studo also allows for physical execution of system and user generated motion, but
requires an explicit connected run request through the gateway.



## Asset flow

Built-in poses, motions, and scenes are immutable source material. User assets
live in dedicated directories:

```text
motion/user/poses/
motion/motions/user/
scenes/user/
```




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

The daemon starts character mode after reboot: configure the servo path, enable
holding torque, move to powered `home`, capture a measured anchor and schedule
idle. Failed or cancelled homing leaves character mode off. Studio Stop lasts
until the next daemon restart; `--character-on-start off` provides torque-off
maintenance startup.

Foreground work outranks speech, reactions and idle. Confirmed voice attention
uses approved small absolute turns and a temporary conversational anchor.
Mechanical `rest` is reserved for shutdown before torque release.


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
