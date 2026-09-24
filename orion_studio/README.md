# Orion Studio

Studio is Orion's desktop app for controls, animation authoring, previews,
settings and diagnostics. It connects to the Pi gateway over authenticated HTTP.
The Pi runs the voice pipeline and owns hardware execution.

Editing changes a local draft or preview. **Publish to Orion**, **Play on Orion**
and the Home controls send explicit requests to the robot. Static previews show
pose and light settings; measured joint telemetry is available in Debug.
Debug's Voice card opens conversation history stored on the Pi. The history
shows recognized requests, replies, tool actions and timing in date order.

## Development

Use Node.js 20.19 or later in the 20.x line, or Node.js 22.12 or later, pnpm,
Rust and the Tauri 2 prerequisites for your platform. Run from the repository root:

```bash
pnpm --dir orion_studio install
pnpm --dir orion_studio test
pnpm --dir orion_studio build
pnpm --dir orion_studio tauri dev
```

`pnpm --dir orion_studio dev` starts the browser frontend at
`http://localhost:1420`. Desktop pairing persistence and native commands require
Tauri. Build and sign desktop packages on their target operating systems.

The Pi installation is described in the [quickstart](../docs/quickstart.md).
Studio uses the Pi's speech service, so opening the desktop app requires no local
speech-model setup. Closing it leaves Orion's services running.

## Connect Studio to the Pi

Install the [Pi services](../docs/quickstart.md#pi-local-voice-and-agent), then
select **Pair Orion** in Studio. Enter the gateway address, normally
`http://orion.local:7447`, and the token from `~/.config/orion/studio-token` on the
Pi. **Pair and remember Orion** verifies the connection and saves the desktop
credential.

Studio reconnects after startup or network loss. **Disconnect** pauses retries
for that session; **Forget Orion on this computer** removes the saved pairing.
**Change address** reuses the saved token and verifies the new address before
replacing the saved connection. A failed check keeps the previous pairing.
Browser development keeps the connection in memory for the current tab.
See [saved pairing](../docs/configuration.md#saved-pairing) for storage and limits.

### Hostname discovery

`orion.local` is discovered through mDNS (Bonjour on macOS, Avahi on the Pi).
The gateway listener and hostname discovery are separate: a running gateway
cannot help a client whose hostname lookup times out.

If the Mac discovers Orion's IPv6 address but not its IPv4 address, lookup can
stall beyond Studio's five-second request timeout. On the Pi, enable
`publish-a-on-ipv6=yes` under `[publish]` in `/etc/avahi/avahi-daemon.conf`, then
run `sudo systemctl restart avahi-daemon`. This publishes the Pi's IPv4 address
over IPv6 mDNS as well; it does not pin an IP address. Keep `use-ipv4=yes` and
`use-ipv6=yes` under `[server]`.

Check from the Mac:

```bash
curl --noproxy '*' --max-time 5 -i http://orion.local:7447/api/v2/status
```

A `401` response without a token confirms
hostname lookup and HTTP connectivity; Studio's pairing token is still required
for authenticated status. The gateway must also support the returned address
family, as configured below.

### Gateway source development

For gateway source development on the Pi, stop its installed service before
starting a second listener on the same port. Supply the existing catalog,
calibration and the trajectory compiler built from the source under test:

```bash
python3 orion_studio/gateway.py serve \
  --bind :: --port 7447 \
  --socket /tmp/oriond.sock \
  --token-file ~/.config/orion/studio-token \
  --project-root /home/mofe/dev/orion \
  --calibration ~/.config/orion/servo_calibration.json \
  --trajectory-compiler /home/mofe/dev/orion/runtime/target/release/orion-trajectory
```

`--bind ::` listens on IPv4 and IPv6 using a dual-stack socket, so either address
returned for `orion.local` can connect. It requires OS IPv6 support. An explicit
IPv4 bind remains IPv4-only; omitting `--bind` listens only on `127.0.0.1`.
Existing Pi deployments preserve installed command arguments: change their
gateway `ExecStart` bind argument to `::` after installing a gateway version
that supports it, then reload systemd and restart the gateway.

The gateway validates supported operations and forwards hardware commands through
the private runtime socket. The [system architecture](../docs/system-architecture.md)
describes its asset and voice-service connections.

## Home

Home shows the listening switch, character status, curated expressions and lamp
controls. **Go to rest** cancels foreground work and follows the calibrated descent.
Measured arrival allows torque release and keeps the light off until confirmed
waking. **Character mode** starts autonomous behavior. Microphone mute is controlled
separately through **Listening**.

Lamp controls select warm white or a custom color and brightness. **Apply** sends
the choice. Speech, scenes and rest can temporarily override visible output;
Home uses the runtime's effective light power to show the switch state. The 3D
model supports rotation at fixed zoom and displays a static pose and light preview.

The light/dark theme is shared across Home, Animation, the editor and Settings and
is saved on the desktop. Debug mode adds a Diagnostics shortcut on Home.

## Animation library and scene editor

Animation separates the built-in Orion collection from user scenes and poses.
Selecting an asset opens its preview. **Play preview** and **Preview pose** affect
the model; **Play on Orion** and **Go to pose on Orion** request hardware execution.
Movement preview compilation requires a connected runtime and its calibration.
Static pose browsing works offline.

A pose defines all five joint positions. A motion describes the journey between
positions, including travel, holds and arrival behavior. A scene combines motion,
lighting and sound. For example, a left-facing pose supplies the destination, a
look motion adds anticipation and settling, and a scene adds a light or cue.
See the [asset reference](../docs/motion-assets.md) and [scene format](../scenes/README.md).

**Create scene** starts from a scene copy or pose and opens the editor. Drafts save
locally and survive restarts. **Save** retains the draft; **Publish to Orion** sends
its scene and owned dependencies to the Pi. Published custom content must be
updated before changed poses can run on Orion. Storage errors remain visible and
prevent leaving an unsaved draft.

The preview and timeline share the editor workspace. Motion, Light and Sound are
collapsible tracks. Drag the playhead to scrub, or use its arrow-key controls.
Movement clips follow the preceding movement by default. Reordering compiles a
continuous sequence; existing explicit gaps remain until reordering. Playback
and publication wait for compilation to finish.

Light and sound can use elapsed time or a movement marker. The editor resolves
these against compiled timing. **Delay** adds a hold after a pose or waiting time
before light or sound. Choose the resulting delay component to change its duration.
Sound durations come from WAV metadata, and sound clips queue within their track.

Right-click a movement and choose **Split into components** to expose its poses and
nonzero delays. Components retain their shared movement compilation and smooth
transitions. Edit or delete them in the inspector; deleting a pose also removes
its attached delay. Split metadata stays in the local draft and is omitted from
published scenes. Shift+F10 opens the same menu for keyboard users.

**Edit pose** opens calibrated joint controls for the preview. **Complete edit**
stores a pose owned by that scene. These poses stay outside the standalone pose
library. Relative components are converted to absolute poses from their compiled
positions when edited. Renaming a scene updates its owned references.

Lighting offers Orion presets and custom Constant, Pulse, Breathe and Fade effects.
Each custom stage selects warm white or a hue and brightness. Choosing a preset
clears custom stage overrides. The browser and runtime use the same stage curves;
physical brightness and clearance still require checks on the robot.

**Delete scene** asks for confirmation. Deleting a draft removes its local copy.
Deleting a published user scene checks its content revision and removes its owned
poses and movements through the gateway. Standalone poses and sounds remain
available. A rejected catalog reload restores the previous files. Built-in scenes
remain read-only.

## Voice observation

The Pi coordinator runs Qwen, Codex and the selected Piper Alba or Pocket model,
uploads response audio to the local gateway and waits for `oriond` playback
completion. Studio polls the gateway for
voice status, transcripts, models and timing.

A voice preset change applies to the next reply without restarting the agent or
ASR. Other model changes restart the coordinator and speech workers. An idle
restart can preserve the conversation; restarting the whole service starts fresh
context. See [voice settings](../docs/configuration.md#voice-settings).

The [voice architecture](../docs/voice-architecture.md) explains wake verification,
continued command capture, response buffering and the follow-up window. A generated
first chunk does not establish audible response time; the coordinator may buffer
the full response when synthesis is slower than playback.

## Settings and Debug

Settings stores appearance, preview sound, reduced UI motion and debug visibility
locally. Voice models, Pocket voice presets, alarm and timer sound choices, personality
and memories save on the Pi. **Voice and sounds** saves each selection automatically:
the default Pocket voice applies to the next reply when Pocket is selected, and
each alarm or timer sound
applies when its next alert starts. An alert already ringing keeps its sound. Sound
choices come from the connected runtime; an older runtime requires an update before
these selectors become available.
Codex model and effort choices come from the active runtime's advertised catalog.
Pi model paths are displayed as read-only. Other model changes require listening
to be muted. Saving while voice status reports an error first attempts to mute,
then retries startup.

Debug shows the voice session, transcripts, model details and stage timings. It
also displays joint position, velocity, current, voltage and temperature when
reported by the runtime. `/api/v2/debug/logs` returns up to 200 journal entries
from `oriond`, `orion-studio-gateway` and `orion-listener`. The gateway account
needs journal access. For voice-host startup and model errors, inspect
`journalctl -u orion-voice-stack` on the Pi.

Debug's **Turn torque off** requests `release_movement` and refreshes status.
The UI blocks it during character mode or active motion; the gateway also rejects
active motion or scenes. Releasing torque stops the joints holding position.
Support Orion before using this control.

## Pi deployment

Use [Pi deployment](../docs/quickstart.md#deploy-to-the-pi) to build and activate the
matching runtime, gateway, listener and voice host. The deployment preserves saved
settings and the existing asset catalog and checks readiness before accepting the
release. The [motion architecture](../docs/motion-and-animation-architecture.md)
describes the runtime contracts used by previews and hardware execution.
