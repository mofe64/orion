# Orion native control architecture

Orion uses a native Rust process for deterministic physical control. MuJoCo is
owned by the existing Python adapter and connected to the same Rust state
machine through a narrow JSON-lines process bridge. ROS is not part of the
runtime.

```text
motion/config + motion/motions
             |
             +--> runtime/oriond --> rustypot --> physical Orion
             |             |
             |             +--> MuJoCo bridge --> simulated Orion
             |
             +--> motion Python library --> established MuJoCo validation

description/meshes --> description/urdf/orion.urdf
                   +--> simulation/mujoco/robot.xml

scenes --> local scene player --> RuntimeCore movement lifecycle
                            +--> LightingDevice
                            +--> AudioDevice
```

## Shared semantics

Both backends use the same five joint names, named poses, keyframe motions,
coordinate convention, and authored timing. Physical execution additionally
validates every target against the captured calibration file before writing a
servo goal.

The shared motion assets live under `motion/`. They do not know whether the
consumer is hardware or MuJoCo. The Rust daemon parses pose and motion YAML
directly and uses one state machine for `rustypot` hardware and MuJoCo. The
established Python player continues to provide independent
trajectory, settling, and stability validation through `motion/orion_motion`.

## Physical runtime

`runtime/target/debug/oriond --serve` owns the serial connection through
`rustypot` and runs a 50 Hz state/control loop. Local commands use
`/tmp/oriond.sock`:

```text
configure -> configured servo profile, torque off
enable    -> seed measured goals, torque on, holding
goto      -> one quintic pose transition
play      -> authored transitions and holds
stop      -> stop active motion while retaining holding torque
disable   -> cancel motion and turn torque off
```

Every accepted `goto` or `play` is one ephemeral execution with a daemon-local
`run_id`. The active execution reports `executing` while trajectory commands are
being produced and `settling` while measured final position and velocity are
checked. It then becomes `completed`, `timed_out`, or `cancelled`. The state
snapshot retains only the active execution and most recent terminal result, so
an agent can correlate asynchronous acknowledgement without introducing a
movement database.

The future behaviour/voice agent owns semantic selection—such as choosing an
authored expressive motion in response to speech—but never writes joint or
servo commands directly. It submits the motion name to `oriond`, keeps the
returned `run_id`, and observes that execution through the local API.

## Multimodal scene layer

The portable `scenes/` library coordinates semantic motion references, RGBW
lighting states, and named audio cues. `runtime/src/scene.rs` validates those
references and dispatches them from one monotonic local clock. It implements
its motion boundary through `RuntimeCore`: the scene receives the same
ephemeral movement run ID and waits through `executing` and `settling` before
declaring completion.

Lighting and audio are narrow device traits. Recording implementations make
scene timing deterministic on a development machine. The physical Pi 5
NeoPixel and ReSpeaker implementations remain separate adapters, so neither a
scene nor the future agent can write GPIO, ALSA, or servo registers directly.
The scene player remains library-level and is not yet exposed as an `oriond`
scene command. Physical Pi 5 RGBW output is available through the direct
`--light`, `--light-pixel`, and `--lights-off` commissioning commands; daemon
ownership is the next integration boundary.

The authoritative physical calibration remains outside the repository at
`~/.config/orion/servo_calibration.json`. The tracked copy under
`simulation/mujoco/config/` provides reproducible simulator limits and is
checked by hash.

## Robot description

`description/urdf/orion.urdf` contains backend-neutral kinematic, visual,
collision, and inertial data. `description/meshes` is the only mesh source.
MuJoCo keeps simulator-specific actuators, contact, references, and physics in
`simulation/mujoco/robot.xml` while using those shared meshes.

## Safety boundary

The Rust runtime enforces captured physical position limits. Dynamic limits
in `motion/config/motion_limits.yaml` remain validation evidence for offline
tools and MuJoCo; the currently requested physical duration determines the
daemon's trajectory rate. Motion timing must therefore be validated on the
assembled lamp before it becomes a production behaviour. The Rust transport,
named poses, authored motions, stop, rest, and disable paths have been validated
on the assembled Orion hardware. Ephemeral run IDs and measured settling have
been verified in deterministic tests and MuJoCo; their completion tolerances
still need one physical Pi run before being treated as tuned hardware values.
