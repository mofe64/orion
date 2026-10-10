# Orion Studio

Studio is Orion's desktop app for controls, animation authoring, previews,
settings and diagnostics. It connects to the Pi gateway over authenticated HTTP.
The Pi runs the voice pipeline and owns hardware execution.

Editing changes a local draft or preview. **Publish to Orion**, **Play on Orion**
and the Home controls send explicit requests to Orion. Static previews show
pose and light settings; measured joint telemetry is available in Debug.
Debug's Voice card opens conversation history stored on the Pi. The history
shows recognized requests, replies, tool actions and timing in date order.

The top-bar pill reports the connection. Orion activity shows playback progress
and **Cancel**, without internal run IDs. Control feedback appears beside the
control that requested it and clears when you leave the page or the connection
changes. Studio has no global status bar.

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
Browser development supports previews and tab-local pairing. Voice, agent,
personality and memory settings require the desktop app and a connected Orion;
Settings explains this prerequisite without offering native-command retries.

The Pi installation is described in the [quickstart](../docs/quickstart.md).
Studio uses the Pi's speech service, so opening the desktop app requires no local
speech-model setup. Closing it leaves Orion's services running.

## Connect Studio to the Pi

Install the [Pi services](../docs/quickstart.md#prepare-a-new-pi), then
select **Connect Orion** in Studio. Enter the gateway address, normally
`http://orion.local:7447`, choose **Get code**, enter the code spoken by Orion
and choose **Pair**. Studio verifies the connection and saves the desktop
credential. **Use a token instead** opens manual token entry. See
[Studio gateway](../docs/configuration.md#studio-gateway) for code limits and the
journal fallback.

Studio reconnects after startup or network loss. **Disconnect** pauses retries
for that session; **Forget this Orion on this computer** removes the saved pairing.
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

Home shows Orion’s mode, listening, Expressions and lamp controls after connecting.
When disconnected, it shows one **Connect Orion** action and a compact placeholder;
Animation remains available for scenes and previews.

Choose **Idle mode** for autonomous movement with timed rest, or **Lamp mode** to
stay on with animations. **Pause mode** and **Resume mode** control automatic
movement without changing the chosen mode. Settings shows the same mode label and
links to Home. **Go to rest** is a separate action: it cancels foreground work and
follows the calibrated descent. Measured arrival allows torque release and keeps
the light off until confirmed waking. Microphone mute is controlled separately
through **Listening**. Timers show time remaining and the local clock time.

Lamp brightness and color apply on release; changes within 150 ms combine into
one command. Sliders retain keyboard focus while a request is running; the latest
released setting applies after that request finishes. Choosing a mood also applies
it. These changes turn the lamp on;
rest keeps its light off. Navigation or a connection change cancels pending
commits. The switch applies immediately and uses runtime light power when reported.
Orion does not report brightness, so the single slider shows **Not set in this
session** until the owner chooses or successfully sets a value. Speech, scenes and
rest can override visible output. The 3D model supports rotation at fixed zoom and
displays a static pose with a neutral light until a lamp setting is accepted.

Expressions shows up to six built-in gestures and published owner scenes. When
both collections are large, three places go to each; **See all** opens Animation.
Scene, speech and foreground movement gates still apply.

Appearance in Settings is the single light/dark theme control. The theme is shared
across Home, Animation, the editor and Settings and saved on this computer. Debug
mode adds a Diagnostics shortcut on Home.

## Animation library and scene editor

Animation separates the built-in Orion collection from user scenes and poses.
Asset names use sentence case throughout Studio. Selecting an asset opens its preview.
The Poses list and create dialog omit calibration references, shutdown-only poses
and built-in transition sub-poses. Those assets remain available to movement
compilation and in-scene editing. Built-ins use owner descriptions supplied by
Studio; unknown built-ins show no description. User descriptions remain visible.
The YAML descriptions remain engineering records. **Preview** and **Preview pose** affect
the model; **Play on Orion** and **Go to pose on Orion** request hardware execution.
Movement preview compilation requires a connected runtime and its calibration.
Static pose browsing works offline. **Movement only** plays movement without light
or sound. **Return to home pose** is the single return-home playback action; the
built-in return-home scene is omitted from the browsing list.

A pose defines all five joint positions. A motion describes the journey between
positions, including travel, holds and arrival behavior. A scene combines motion,
lighting and sound. For example, a left-facing pose supplies the destination, a
look motion adds anticipation and settling, and a scene adds a light or cue.
See the [asset reference](../docs/motion-reference.md) and [scene format](../scenes/README.md).

**Create scene** starts from a scene copy or pose and opens the editor. Drafts save
automatically on this computer and survive restarts. The visible save state
reports **Saving…**, **Saved on this computer** or **Couldn't save**. Scene names
use raw draft text while focused and sentence case after editing. They commit on
blur or Enter: a name is required, must be unique and cannot replace a
built-in scene. **Publish to Orion** sends the scene and its owned dependencies
to the Pi. Its disabled state explains whether Orion is disconnected or still
calculating timing. Published custom content must be updated before changed poses
can run on Orion. Storage errors remain visible and
prevent leaving an unsaved draft. Publishing remains available to preserve the
scene on Orion when local storage fails.

Preview preparation, publication and playback failures show a plain sentence
beside their controls. **Technical details** keeps the gateway diagnostics
collapsed until opened.

The scene editor fits the desktop viewport. Its Movement, Light and Sound summaries
stay visible at 1440×900 and 1280×800, including while timing is calculated.
Expanding a track keeps all three summaries visible above its detailed items.
The details scroll in a separate area below that opaque overview without hiding
Light or Sound. The 3D preview retains at least 10rem of height when tracks expand.
The selection inspector has one scroll area. Drag the playhead to scrub, or use
its arrow-key controls. Short clips use ellipses; select a clip to read its full
name above the tracks. **Preview from** changes only the model's starting pose;
Orion starts from wherever it is. All model views use **Drag to rotate** guidance.
The editor authors scenes, including scene-owned poses and movements; standalone
pose and movement authoring is unavailable. Movement clips follow the preceding
movement by default. Reordering compiles a
continuous sequence; existing explicit gaps remain until reordering. Playback
and publication wait for compilation to finish.

Light and sound can use elapsed time or a movement marker. The editor resolves
these against compiled timing. The selected movement or pose offers **Add delay**
in the inspector. Light and sound use **Delay before**. Choose a delay in the
track to change its duration.
Sound durations come from WAV metadata, and sound clips queue within their track.

Choose **Split into poses** in the movement inspector, or use the right-click
menu, to expose its poses and nonzero delays. Components retain their shared
movement compilation and smooth transitions. Edit or delete them in the
inspector; deleting a pose also removes
its attached delay. Split metadata stays in the local draft and is omitted from
published scenes. Shift+F10 opens the same menu for keyboard users.

**Edit pose** opens calibrated joint controls for the preview. **Complete edit**
stores a pose owned by that scene. These poses stay outside the standalone pose
library. Relative components are converted to absolute poses from their compiled
positions when edited. Renaming a scene updates its owned references.

Lighting offers Orion presets and custom Constant, Pulse, Breathe and Fade effects.
Each custom stage selects warm white or a hue and its own brightness.
**Overall brightness** scales every stage together without changing their
relative levels. Choosing a preset clears custom stage overrides. The browser
and runtime use the same stage curves;
physical brightness and clearance still require checks on Orion.

**Delete scene** asks for confirmation and shows **Scene deleted** beside the
library after success. A failed deletion keeps the dialog open with a plain
message and collapsed **Technical details**. Deleting a draft removes its local copy.
Deleting a published user scene checks its content revision and removes its owned
poses and movements through the gateway. Standalone poses and sounds remain
available. A rejected catalog reload restores the previous files. Built-in scenes
remain read-only.

## Voice observation

The Pi coordinator runs Qwen, Codex and Piper Alba Medium,
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

Settings has one ordered column: **Orion**, **Personality and memory**, **Voice**,
**Studio**, then **Developer tools**. Orion contains mode, listening and alert sounds.
Appearance, preview sound, reduced UI motion and debug visibility save on this
computer with inline **Saved** feedback. Listening and alert sounds apply on the Pi
immediately; each sound choice applies when its next alert starts. An alert already
ringing keeps its sound. Sound choices come from the connected runtime; an older
runtime requires an update before these selectors become available.

Personality choices save automatically after a short pause, preserving newer
choices made during a save. Pending saves finish when you leave Settings;
a connection change clears the old profile and cancels deferred changes. Valid
memory text saves when leaving its field; **Cancel**
discards an unsaved memory draft. Deletion still requires confirmation. Changes
start a fresh conversation on Orion’s next request, and failures preserve the draft.
Relevant memories are sent to Codex when used; editing or deleting does not erase
information already sent to Codex.

**Voice** is the only section with a Save button. Codex model and effort choices
come from the active runtime’s advertised catalog, using friendly names while
preserving model IDs. Saving turns listening off and restarts the voice coordinator;
turn listening on when Orion is ready. Saving while voice status reports an error
first attempts to mute, then retries startup. Alba is the fixed British English voice.

Until saved voice settings or the profile loads, their controls show a loading
message or a plain failure with collapsed **Technical details**. Settings never
offers local Mac model folders. Browser development shows the desktop-app and
connection prerequisite. See [voice settings](../docs/configuration.md#voice-settings).

**History** is visible in navigation without debug mode and shows saved conversations,
voice steps and times. It requires the desktop app and a connected Orion. Tool
request and result JSON stays collapsed. Debug retains its history link.

Debug shows voice activity reported by Orion's Pi, transcripts, model details
and stage timings. It
shows plain labels beside raw runtime, character, clip, idle-category and playback
state tokens. It also displays joint position, velocity, current, voltage and temperature when
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
release. The [motion architecture](../docs/motion-architecture.md)
describes the runtime contracts used by previews and hardware execution.
