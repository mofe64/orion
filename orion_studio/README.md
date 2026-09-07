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

Home provides a listening switch, character status, curated expressions, and three
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

Scene movements added in Studio automatically follow the preceding movement.
Use **Move earlier** and **Move later** to arrange them without start timestamps;
reordering makes the movement track a continuous sequence. Existing explicit gaps
remain until reordering, and overlapping clips shift forward after compilation.
Timing uses full compiler precision. Runs and publishing wait for the current
sequence to finish compiling. Automatic placement is saved in Studio drafts;
published v2 files contain resolved numeric starts. Marker-linked light and sound
follow their movement; explicitly timed effects keep their authored timestamps.

The preview distinguishes a static pose, compilation in progress, a failed
compile, and a compiled preview. Compiled movement uses the Rust trajectory
compiler and connected calibration; it does not establish physical clearance.
Unresolved timeline events stay in **Calculating scene timing**. Each resolved event
has a separate selectable row; zoom expands the time scale.

Robot activity shows accepted run IDs, progress, terminal results, and a
run-specific cancel action. Diagnostics contains runtime and calibration details;
seeds and simulated reactions belong to developer tools. Both screens load the
3D renderer on demand, render on changes, and release GPU resources on exit.
Create retains its orbit, zoom, and pan controls independently of Home. Its
floor grid is centered on the stationary base and aligned with the base’s
edges; it is a visual reference, not a change to robot coordinates or calibration.

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
responses with Chatterbox. The top-level [`orion-agent` library](../agent/README.md)
is compiled into Studio and owns the Codex conversation separately from the
[`orion-coordinator`](../coordinator/README.md) pipeline. The coordinator calls
the agent directly and owns the top-level Python speech worker. Idle voice-model
reloads preserve the conversation.
Playback is Pi-owned:

```text
Chatterbox signed 16-bit pulse-code modulation (PCM16)
  -> Rust coordinator encodes mono 24 kHz RIFF/WAV chunks
  -> authenticated /api/v2/speech/stream and run-scoped chunk/end requests
  -> oriond/ReSpeaker playback
  -> energy-driven speaking motion + warm red-green-blue-white (RGBW) light
  -> terminal status
  -> Rust coordinator acknowledges completion to the Pi listener
```

The Rust coordinator polls the run through queued, playing, and terminal states
and acknowledges completion to the Pi listener only after playback completes. Cancellation
is run-scoped. The runtime deletes spool files after completion, cancellation,
or failure. Conversational speech requires the Studio processing station;
there is no Pi-local Piper fallback. See the
[voice architecture](../docs/explanation/voice-architecture.md#streaming-replies-and-timing)
for buffering and streaming behaviour.

Prepare the optional Apple Silicon voice models separately:

```bash
cd speech
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

## Animation library and scene editor

**Animation** opens a scene/pose library with separate **Orion collection** and
**My scenes / My poses** groups. Selecting an item never sends a robot command.
**Play preview / Preview pose** affects the model; **Play on Orion / Go to pose on
Orion** explicitly requests hardware playback. **Full scene** previews movement,
lighting colours/intensity/fades, and local audio cues; **Movement only** omits
scene lighting/audio events. The runtime's normal finish/idle lighting policy
still applies after hardware playback. The visual light is an approximation of
runtime effects, not a photometric simulation.

**Return to home pose** plays the existing `return_home` scene on Orion only,
and is disabled while disconnected, busy, or reporting `home_idle`. To view the
home pose in the model, select **Home** under **Poses** and use **Preview pose**.
Studio does not receive measured joint telemetry through this status API.
Scene preview compilation requires a connected
Orion, while static pose browsing works offline.

**Create scene** starts from a scene copy or pose, and opens the separate editor.
System scenes remain intact. User drafts survive restarts and appear in the user
collection. Pose-based scenes get an internal starting movement; publishing the
scene first publishes any new movement dependencies. The editor retains event
and timeline controls and supports saving a copied destination pose. The theme
preference applies to every screen and persists locally. Diagnostics is a Home
quick action. All views illuminate the diffuser with a light parented to its mesh,
so the beam follows the lamp head.

**Delete scene** follows **Edit scene** for user scenes and asks for confirmation.
Deleting a local draft removes it from this device. Deleting a published scene
requires a connected gateway advertising `scene_library.delete`; it removes the
scene from Orion and its local draft, including scene-owned poses and movements.
Standalone library poses and sounds remain intact. The gateway
checks the scene revision before deletion and restores the file if catalog reload
fails. System scenes cannot be deleted from Studio.


The scene editor places its editable name, **Save**, and **Publish to Orion** at
the top. A labelled status icon indicates local save state. Playback controls sit
below the model in a separate layout row. On desktop, the preview and timeline
share the available screen height; long timelines and selection panels scroll
independently. Right-click a movement and choose **Split into components** to
replace its clip with individual pose and nonzero delay clips within the Motion
track. Collapsed, the track keeps all clips on one row. Expanding Motion reveals
its grouped component rows.
Select a component to edit it in the inspector; right-click to delete that component.
Deleting a pose also removes its attached delay. Split state persists in the draft.
Keyboard users can open the same menu with Shift+F10. Internally, components retain
their shared movement compilation so smooth transitions and speed checks remain
intact. Edits use a private movement copy. Component positions use compiled preview
samples (at the preview sampling resolution); before compilation, rows remain
selectable with timing pending. Split metadata is omitted from published scenes.


Motion, Light, and Sound each have a collapsible track. Drag the playhead to scrub;
its grab cursor and keyboard arrow controls support precise positioning. Drag a
clip to reposition it, with a 12-pixel snap threshold at the track start and nearby
clip edges. Components reorder within their parent movement. Media items queue
within their own track; sound durations come from WAV metadata. Movement cue links
remain authored links, while publication uses their resolved, non-overlapping times.
**Starts** explains scene-time, previous-item, and movement-cue timing. **Delay** adds
waiting time to the selected item: a hold after a motion pose, or a wait before a
light or sound. Select the resulting Delay component to change its duration.

The lighting picker separates **Custom effects** (Constant, Pulse, Breathe, Fade)
from **Orion presets**, which always lists all developer-prepared effects. Choosing
a preset clears custom stage overrides. Brightness sliders show percentages.
Each custom stage has Warm white or a
single custom-color hue slider and brightness. Effect explanations expand in place.
The browser and runtime use the same stage curves; existing character presets remain
unchanged until the user selects another effect.

Right-click a pose and choose **Edit pose**. The inspector shows calibrated joint
controls and updates the model without sending hardware commands. **Complete edit**
stores `<scene>_custom_pose_<n>` inside the scene draft. These poses do not appear in
the pose library. Scene renaming updates the embedded pose and movement references;
deleting the scene removes its embedded assets. Relative components are converted
from their compiled positions into scene-owned absolute poses when edited.

Custom pose previews require the updated gateway's optional trajectory `poses`
document. It writes a temporary compiler input, leaving the Pi library unchanged.
Publishing includes a `studio` bundle: the gateway stages its owned pose/movement
files in `_scene_owned` directories and uses `asset reload`. Failed publication
restores the previous files. Published custom scenes must be updated before their
changed poses can play on Orion. Deploy the matching runtime and gateway before
using custom effects or bundled assets on hardware; a frontend update alone is
insufficient.

## Settings and Debug

Settings follows Animation in navigation and shares Home’s light and dark themes.
Studio preferences (appearance, preview sound, reduced interface motion, and debug
mode) persist in local webview storage. Character mode and the listening switch
control Orion through the existing runtime and listener interfaces.

Codex model/effort, Qwen3 ASR and Chatterbox model IDs, optional local weight
folders, and the model download cache save atomically to
`~/.config/orion/voice-settings.json` on the Studio computer. Older reply settings
migrate when loaded. Turn listening off before saving voice changes. Speech settings display the detected local cache snapshots and provide native
folder pickers for weights and the download cache. Cancel keeps the selection;
Use default removes the override. Cached weights are reused, then loaded into
memory when the worker starts; model IDs can check for updated revisions. Local folders
must exist and take precedence over model IDs; weights must match the named engine.
The cache sets `HF_HOME` and `HF_HUB_CACHE` for future worker starts and does not move
existing downloads. Speech engines require Apple Silicon. API-key provider fields
are placeholders: submitting, saving, and sending their contents is disabled.

Enable debug mode to expose Debug navigation and Home’s Diagnostics shortcut.
Debug shows local voice state, transcripts, model details, timing, and runtime joint
position, velocity, current, voltage, and temperature when reported by Orion.
`GET /api/v2/debug/logs` uses the existing gateway authentication and reads only the
latest 200 journal entries for `oriond`, `orion-studio-gateway`, and `orion-listener`.
Install the updated gateway on Orion to use logs; its service account needs journal
read access. Missing journal support or access produces an unavailable message.
The former Voice modal is removed; its controls live in Settings and Debug.

Codex model and effort dropdowns use the coordinator’s advertised catalog; no
manual or assumed options are offered. Speech model information is read-only:
Hub snapshot folders identify their repository, while copied folders may only
identify an architecture from `config.json`. The worker loads selected folders
directly without requiring their original repository IDs.

Debug’s **Turn torque off** uses `release_movement`, refreshes robot status, and
blocks release during character mode or active movement. The gateway independently
rejects active motion or scenes. Releasing torque stops joints holding position;
support Orion before using it.
