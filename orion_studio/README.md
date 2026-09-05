# Orion Character Studio v2

Studio is a dark, accessible creative workspace for everyday Orion owners and
motion builders. It authors pose, motion, and scene v2 assets while the Pi
remains the only hardware authority.

```text
Studio / Chatterbox ── authenticated HTTP v2 ──> Pi gateway
                                                   │ private Unix socket
                                                   v
                                                oriond
                                   motion + character + light + sound
```

Editing is inert. A slider, keyframe, or timeline drag never moves Orion.
Explicit Home controls, **Run on Orion**, and **Publish asset** actions cross
the gateway.

## Home and Create

Home provides voice access, character status, curated expressions, and three
routine controls:

- **Go to rest** cancels active speech or scenes, turns character mode off, and
  follows the runtime's calibrated three-second movement to the rest pose.
  Motors continue holding the pose; this is not a torque-release command.
- **Character mode** starts autonomous character behaviour and restores
  expressive lighting.
- The **Lamp power** switch turns manual light on or off through `oriond`.
  Choose **Warm white** or **Custom color**, set brightness, then select
  **Apply**. Custom color reveals one spectrum slider without numeric color
  fields. Speech and scenes can temporarily take priority over the manual light.

These controls require the updated gateway and runtime on the Pi. Editing a
color or brightness alone does not send a command. The switch shows the last
accepted lamp command in this Home session; the gateway does not report live
lamp state. Failed requests do not change the switch.

Home includes a rotatable 3D model with fixed zoom and no camera panning. The
model shows the attentive pose and the last accepted lamp setting as a preview,
not live robot telemetry. Rotating it never sends a robot command. Home starts
in dark mode; its **Light mode** toggle does not change Create’s appearance.

Create contains three levels of an expression:

- **Pose:** one body position, defined by the five joint angles.
- **Motion:** how Orion travels between positions, including timing, holds,
  anticipation, and settling.
- **Scene:** movement coordinated with lighting and sound. Events can use
  elapsed time or a named motion marker.

For example, a left-facing pose defines the destination; a left-looking motion
adds the expressive journey; a scene adds a light response or sound. Keeping
these separate lets expressions reuse the same poses and motions.

Drafts save on this device per asset and restore when selected again, including
following a restart. **Discard changes** restores the catalog version.
**Publish asset** sends an asset to Orion; edited poses and motions must be
published before running. Browser-storage errors remain visible and prevent
switching away from an unsaved asset.

The preview distinguishes a static pose, compilation in progress, a failed
compile, and a compiled preview. Compiled movement uses the Rust trajectory
compiler and connected calibration; it does not establish physical clearance.
Unresolved timeline events stay in **Awaiting compilation**. Each resolved event
has a separate selectable row; zoom expands the time scale.

Robot activity shows accepted run IDs, progress, terminal results, and a
run-specific cancel action. Diagnostics contains runtime and calibration details;
seeds and simulated reactions belong to developer tools. Both screens load the
3D renderer on demand, render on changes, and release GPU resources on exit.
Create retains its orbit, zoom, and pan controls independently of Home.

## Development

Use Node.js 20 or newer, pnpm, stable Rust, and the Tauri 2 prerequisites for
your platform.

```bash
cd orion_studio
pnpm install
pnpm test
pnpm build
pnpm tauri dev
```

`pnpm dev` runs the UI-only frontend on `http://localhost:1420`. Voice worker
startup and other native commands require Tauri. macOS, Windows, and Linux
packages must be built and signed on their respective target platforms.

## Connect Studio to the Pi

The Pi runs `oriond.service` and `orion-studio-gateway.service`. Create the
private pairing token once:

```bash
python3 orion_studio/gateway.py create-token \
  --token-file ~/.config/orion/studio-token
```

For source development, start the gateway with the Pi calibration and installed
trajectory compiler:

```bash
python3 orion_studio/gateway.py serve \
  --bind 0.0.0.0 --port 7447 \
  --socket /tmp/oriond.sock \
  --token-file ~/.config/orion/studio-token \
  --project-root /home/mofe/dev/orion \
  --calibration ~/.config/orion/servo_calibration.json \
  --trajectory-compiler /home/mofe/dev/orion/runtime/target/release/orion-trajectory
```

In desktop Studio, select **Pair Orion**, enter `http://orion.local:7447` and
paste the token once. **Pair and remember Orion** verifies the robot and saves
the address/token in the OS credential store. Studio reconnects on later
launches and after network loss. **Disconnect** pauses retries for this session;
**Forget Orion on this computer** removes the saved pairing. The browser-only
development UI supports an in-memory connection for the current tab, without
persisting its token. The API accepts semantic
v2 operations only and never exposes arbitrary paths, registers, or joint
streams.

## Studio Voice playback

The Pi owns Rustpotter and microphone capture. Studio receives endpointed
utterances over the local network, confirms them with Qwen, invokes the agent and synthesizes
responses with Chatterbox. Playback is Pi-owned:

```text
Chatterbox signed 16-bit pulse-code modulation (PCM16)
  -> exact mono 24 kHz RIFF/WAV
  -> authenticated POST /api/v2/speech
  -> random Pi spool identifier
  -> oriond/ReSpeaker playback
  -> energy-driven speaking motion + warm red-green-blue-white (RGBW) light
  -> terminal status
  -> Studio playback acknowledgement
```

Studio polls the run through queued, playing, and terminal states and reports
completion to the voice worker only after Pi playback completes. Cancellation
is run-scoped. The runtime deletes spool files after completion, cancellation,
or failure. Pi-local Piper uses the same speech coordinator, so it receives the
same motion and lighting behavior.

Prepare the optional Apple Silicon voice models separately:

```bash
cd orion_studio/voice_worker
uv sync --python 3.12
.venv/bin/orion-voice-models
```

The agent receives confirmed text only. Agent-generated prose cannot issue raw
robot commands. The Pi listener maps confirmed session events to allowlisted
character reactions and optional commissioned attention. Follow
[Pi voice setup](../voice/README.md) before enabling Voice.

## Atomic deployment

`scripts/deploy_pi.sh` validates this Studio build before updating the Pi. The
remote phase returns the running robot to mechanical rest, releases torque,
fast-forwards the selected branch, validates the user asset catalog, builds
runtime and trajectory binaries, installs the Pi Rustpotter environment,
installs all three services, and runs light/audio
plus left/right expressive physical smoke tests. It verifies native wake-model
loading and listener authentication as part of the same command. All components therefore come
from one Git revision.

See the [system architecture](../docs/explanation/system-architecture.md),
[motion architecture](../docs/explanation/motion-and-animation-architecture.md),
and [scene reference](../scenes/README.md).

See the [Studio home audit](../docs/project/studio-home-audit-2026-09-04.md) for
the original findings and validation of their fixes.
