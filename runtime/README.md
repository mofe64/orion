# Orion Rust runtime

`oriond` owns servo communication, light output and speaker playback. Clients request named poses, motions, scenes or supported controls through its Unix socket. Hardware and MuJoCo use the same trajectory compiler.

The runtime owns the [shared mind state](../docs/motion-architecture.md#shared-mind-state). The `status` command includes `mind`.
A separate worker saves hardware state. MuJoCo keeps its mind state in memory.

## Build and test

Run these steps from the repository root. Simulation also requires the [MuJoCo Python environment](../simulation/mujoco/README.md#python-environment).

1. Build the runtime and trajectory compiler.

   ```bash
   cargo build --manifest-path runtime/Cargo.toml --locked
   ```

2. Run all runtime test targets.

   ```bash
   cargo test --manifest-path runtime/Cargo.toml --all-targets --locked
   ```

3. Check Rust formatting.

   ```bash
   cargo fmt --manifest-path runtime/Cargo.toml -- --check
   ```

4. Run Rust documentation tests.

   ```bash
   cargo test --manifest-path runtime/Cargo.toml --doc --locked
   ```

5. Read the complete client and daemon option list.

   ```bash
   runtime/target/debug/oriond --help
   ```

## MuJoCo daemon

1. Start V2 simulation without automatic character movement.

   ```bash
   runtime/target/debug/oriond --serve --backend mujoco --hardware v2 --start-pose home --character-on-start off
   ```

2. Use client commands from another terminal.

The default socket is `/tmp/oriond.sock`. `--socket` selects another socket for both daemon and clients.

## Physical hardware

Use the [V2 setup procedure](../docs/hardware-versions.md#calibrate-v2-first) before enabling motion. Only one process can own the servo bus.

1. Stop installed services before running a source daemon.

   ```bash
   sudo systemctl stop orion-voice-stack orion-vision orion-studio-gateway orion-listener oriond
   ```

2. Read startup positions without enabling torque.

   ```bash
   runtime/target/debug/oriond --check --hardware v2
   ```

3. Start the hardware daemon with automatic character movement disabled.

   ```bash
   runtime/target/debug/oriond --serve --backend hardware --hardware v2 --character-on-start off
   ```

The daemon initially observes joint feedback. The startup check allows a 20-count margin around profile limits. Activation clamps initial goals into range. Later command limits remain strict.

`--port` selects a servo adapter. Without it, Orion requires exactly one detected adapter. `--baud-rate` defaults to 1000000.

## Control the daemon

The examples use the debug client built above. The installed `oriond` launcher accepts the same options.

1. Read runtime status.

   ```bash
   runtime/target/debug/oriond --status
   ```

2. Apply runtime configuration.

   ```bash
   runtime/target/debug/oriond --configure
   ```

WARNING: Enabling torque can move loaded joints. Keep fingers and objects outside the movement area.

3. Enable torque.

   ```bash
   runtime/target/debug/oriond --enable
   ```

4. Move to the named home pose.

   ```bash
   runtime/target/debug/oriond --goto home --duration 3.0 --wait
   ```

5. Play a named motion.

   ```bash
   runtime/target/debug/oriond --play look_at_left_expressive --wait
   ```

6. Cancel active movement when necessary.

   ```bash
   runtime/target/debug/oriond --stop
   ```

A movement passes through these states:

```text
executing → settling → completed
    │           └──→ timed_out
    └──────────────→ cancelled
```

`--wait` accepts `--goto`, `--play` and `--run-scene`. Completion requires measured settling after trajectory samples end. Stop cancels motion and keeps the existing torque state.

## Normal shutdown

WARNING: Disabling torque removes joint support. Complete the rest movement before releasing torque.

1. Move to mechanical rest.

   ```bash
   runtime/target/debug/oriond --goto rest --duration 3.0 --wait
   ```

2. Disable torque after successful completion.

   ```bash
   runtime/target/debug/oriond --disable
   ```

The character rest coordinator performs descent and torque release together. Studio's **Go to rest** uses that coordinator. A direct pose command requires the separate release above.

## Lighting, audio and scenes

RGBW means red, green, blue and white channels. Each channel accepts an integer from 0 to 255. Pixel indices depend on the selected hardware profile.

1. Set a low warm light level.

   ```bash
   runtime/target/debug/oriond --hardware v2 --light 8 3 0 20
   ```

2. Turn all light channels off.

   ```bash
   runtime/target/debug/oriond --hardware v2 --lights-off
   ```

3. Play a local cue.

   ```bash
   runtime/target/debug/oriond --play-cue acknowledge_warm
   ```

4. Run a combined scene.

   ```bash
   runtime/target/debug/oriond --run-scene acknowledge_left --wait
   ```

5. Read scene status.

   ```bash
   runtime/target/debug/oriond --scene-status
   ```

Speech status and cancellation use separate client options.

1. Read speech playback status.

   ```bash
   runtime/target/debug/oriond --speech-status
   ```

2. Stop current speech playback.

   ```bash
   runtime/target/debug/oriond --stop-speech
   ```

See [scene fields](../scenes/README.md) and [speech animation](../docs/speech-animation-runtime.md).

## Capture-time camera rays

V2 keeps measured pose history for camera transforms. `camera-rays` converts camera rays into base-frame bearings using the capture timestamp. A bearing gives a horizontal and vertical direction.

The request uses exposure-midpoint `CLOCK_BOOTTIME` nanoseconds. The ray frame uses right, down and forward axes. Missing or stale pose history returns an error. `--camera-urdf` selects the camera geometry.

## Bearing gaze

V2 gaze receives target bearings through runtime commands. It compiles movement through the joint profile and respects foreground motion ownership.

`--gaze on` enables gaze with V2 serving mode. `--gaze off` suppresses it. Stop, rest and disable prevent autonomous movement. Explicit character start restores normal operation. Gaze resume requires restored power after disable.

Desk-light aiming uses runtime planning and joint limits. A successful aim becomes the held work anchor. Explicit movement or character restart releases that hold.

## Camera viewer indicator

The V2 ring shows a steady cyan band while the gateway holds viewing permission. Snapshots keep the indication for at least three seconds. Rest suppresses viewing and the indicator.

Viewing permission expires unless the gateway renews it. Detection polling cannot create or extend viewing permission.

## Runtime source layout

| Directory | Function |
| --- | --- |
| `src/app/` | CLI, daemon startup and request handling |
| `src/control/` | Runtime state, movement lifecycle and routines |
| `src/devices/` | Servo, lighting, audio and simulator drivers |
| `src/expression/` | Character, gaze, speech and idle planning |
| `src/motion/` | Pose loading, kinematics and trajectory compilation |
| `src/ipc/` | Local socket transport |

See [motion controls](../docs/motion-reference.md), [configuration](../docs/configuration.md) and [tracking tools](../docs/servo-tracking.md).
