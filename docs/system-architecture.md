# Orion system architecture

## System boundary

Orion's Raspberry Pi owns movement, lighting, and audio capture and playback.
Orion Studio runs on an external computer and provides asset authoring, speech
recognition, the agent, and speech synthesis. Control commands and reply audio
follow this path:

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

The hardware and MuJoCo backends use the same `oriond` movement lifecycle and
trajectory compiler. Each backend supplies its own device feedback.

## Authority boundaries

### `oriond`

`oriond` owns the servo bus, RGBW light output, and ReSpeaker playback. It applies
calibration, compiles smooth trajectories, checks movement completion, and
coordinates character behavior, scenes, speech, and automatic rest. Every motion
request passes through its limits and ownership checks.

### Studio gateway

The gateway authenticates Studio requests and exposes versioned operations for
assets, movement, lighting, speech, and status. It translates accepted requests
into commands on the Pi's private `/tmp/oriond.sock`. The gateway uses a bearer
token over HTTP; deployment currently assumes a trusted local network.

### Orion Studio

Studio owns asset browsing, editing, preview, and connection state. Its Rust
voice coordinator runs transcription and synthesis jobs through a Python speech
worker and sends confirmed commands to the Rust agent. The agent can search the
web, manage explicitly requested memories, and request validated lamp changes.
Lighting requests pass through the coordinator, gateway, and `oriond`.

Studio can request execution of built-in and user-authored motion through the
gateway. The agent's available tools do not include motion control. The
[voice architecture](voice-architecture.md) describes its conversation and
playback lifecycle.

### Pi listener

The listener owns microphone capture, Rustpotter wake detection, and utterance
endpointing. It sends captured utterances to Studio over an authenticated
WebSocket and forwards voice events to `oriond` on the local socket. Capture
runs independently of the motor loop and remains available during mechanical
rest unless the microphone is muted. Studio must be connected to confirm a wake
through speech recognition.

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
They finish after their dispatched work reaches a terminal result. Runtime
status exposes the active movement and most recent terminal movement. Run IDs
and retained results reset when the daemon restarts.

By default, daemon startup configures the servos, enables holding torque, and
moves to `home`. Successful measured arrival establishes the character's anchor
and starts idle behavior. Explicit foreground work takes priority over speech,
reactions, and idle. Confirmed voice attention uses a small approved turn and
holds a temporary conversational anchor.

Studio's **Stop character** cancels owned work, returns home, and leaves torque
holding. Character behavior resumes after an explicit start or the next daemon
startup with character mode enabled. `--character-on-start off` starts the daemon
with torque disabled for maintenance. Voice capture has its own mute control.

## Automatic rest and waking

Orion returns to mechanical rest after ten minutes without a wake confirmation
accepted through speech recognition. Successful initial homing arms the timer. Each current wake
session can reset it once; raw candidates, rejected wakes, repeated
confirmations, and ordinary animation leave the deadline unchanged.
[Configuration](configuration.md#pi-listener-and-character-startup) describes
the timeout option.

An active confirmed conversation, including its listening window and continuation
turns, defers rest. Foreground motion, scenes, queued or playing speech, and the
character's speech settling also defer it. Continuation speech preserves the
deadline set by the last confirmed wake. Once the body is available, an expired deadline
can start rest immediately. Background idle and unconfirmed thinking movements
can yield to rest.

`RestCoordinator` tracks the rest movement by run ID. It releases torque only
after that movement reports measured completion. A timeout, cancellation,
missing completion result, or failed torque release enters `fault`. The runtime
reports the error and awaits explicit recovery. A home movement that fails or
is cancelled during waking also enters `fault` and blocks automatic waking and
reply playback.

A confirmed wake from `resting` enables torque and starts home movement. A wake
confirmed during descent lets rest finish, then starts home while retaining
torque. Cancelling that pending voice session allows the descent to finish in
rest. After home completes, Orion applies the latest listening or thinking
state and may turn toward fresh direction evidence. The
[voice wake sequence](voice-architecture.md#confirmed-waking) describes capture
and reply ordering during this movement.

Studio's **Go to rest** cancels foreground work and starts the same rest and
torque-release sequence immediately. Explicit Stop, `disable`, and maintenance
startup disable automatic waking; an explicit character start rearms the policy.
The lower-level `goto rest` command only moves the body and requires a separate
torque-release command after verified arrival.

## Light and rest status

At the start of descent, the runtime freezes the visible light frame and fades
it to black over one second. Its light-device wrapper suppresses subsequent
output from character animation, scenes, voice feedback, and manual lamp
commands while descending, resting, or in a rest lifecycle fault. Lamp
preferences remain stored. Waking releases this output gate so expressive
lighting can resume.

The gateway includes runtime rest status in `/api/v2/status`. Its state is
`disabled`, `awake`, `going_to_rest`, `resting`, `waking`, or `fault`. Status also
reports the timeout, last accepted confirmation, remaining time, tracked rest
movement, effective light power, and any transition error. Remaining time can
be zero while active work defers rest. Studio displays resting, waking, and
fault information and uses effective light power for the lamp switch.

## Validation limits

Component tests check timing, completion rules, and injected failures. Command
tests check validation and session ownership. Daemon integration tests exercise
the actual socket and server loop with a deterministic driver that follows
commanded positions. See the [runtime test instructions](../runtime/README.md#build-and-test).

The native MuJoCo rest test exercises a known failure: arm/base contact leaves
the shoulder about 0.218 radians short of the calibrated rest pose. The movement
times out and torque stays enabled. Successful tests with the deterministic
driver establish software coordination; physical acceptance still requires the
assembled robot to reach supported rest, release torque, return home, and retain
speech captured while waking. Direction, light fading, and cue/servo pickup by
the microphone also require physical checks.
