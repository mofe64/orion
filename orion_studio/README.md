# Orion Studio

Orion Studio is the cross-platform desktop workspace for previewing and
authoring Orion scenes, then submitting named work to the robot.

```text
Tauri + React Studio                    Raspberry Pi
┌─────────────────────────┐  HTTP v1   ┌──────────────────────────┐
│ URDF preview + timeline │───────────▶│ gateway.py               │
│ project + Pi scene views│  token     │ semantic + scene adapter │
└─────────────────────────┘            └────────────┬─────────────┘
                                                   │ private Unix socket
                                                   ▼
                                                oriond
                                        sole hardware authority
```

Studio never opens a servo, light, or audio device. The Pi gateway accepts
only named pose, motion, scene, speech, status, and run-scoped cancellation
operations. `oriond` remains responsible for validation, interpolation,
lifecycle state, and all physical execution.

## Desktop development

Install Node.js 20 or newer, pnpm, the stable Rust toolchain, and the Tauri 2
prerequisites for your OS. In brief:

- macOS: Xcode Command Line Tools.
- Windows: Microsoft C++ Build Tools and WebView2.
- Linux: WebKitGTK and the other Tauri system packages for your distribution.

On Ubuntu/Debian, install the current Tauri prerequisites with:

```bash
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

Then run:

```bash
cd /path/to/orion/orion_studio
pnpm install
pnpm tauri dev
```

For UI-only work, `pnpm dev` opens the same frontend at
`http://localhost:1420`. Useful checks are:

```bash
pnpm test
pnpm build
```

The production bundle is configured for macOS, Windows, and Linux. Linux uses
a platform overlay that produces `.deb` and `.rpm` installers:

```bash
pnpm tauri build
```

macOS and Windows packages still need to be built and signed on their target
operating systems. Their `.icns` and `.ico` assets remain in the shared bundle
manifest.

During source development, Studio resolves the Orion project from its Tauri
crate location. A packaged or relocated build can point at a checkout explicitly:

```bash
ORION_PROJECT_ROOT=/path/to/orion pnpm tauri dev
```

## What Studio can do now

- Load Orion's real URDF and STL meshes into an orbitable 3D preview.
- Browse the existing scenes, named poses, and authored motions directly from
  the Rust/YAML project assets.
- Create calibration-bounded named poses with five joint sliders. The sliders
  use the running robot's commissioned limits while connected and the tracked
  operational limits while offline; editing a slider never moves hardware.
- Build named motions as ordered pose keyframes with transition and hold times.
  Studio previews the same quintic blend shape that `oriond` executes.
- Preview pose and motion timing locally with the same quintic blend shape used
  by the runtime.
- Scrub and play a multi-lane scene timeline for movement, RGBW lighting, and
  queued local audio cues.
- Insert pulse and two-cycle breathe lighting templates. Studio expands them
  into editable version-1 RGBW fade events, so preview and hardware use the
  existing runtime lighting path rather than a separate effects engine.
- Add, select, drag to retime, edit, and delete scene events, and edit the
  scene description.
- Append every new clip to the end of its lane, right-click clips for delete
  and delay actions, and split motions or nested scene clips into editable
  pose/light/audio parts. Motion holds remain visible as editable Delay clips.
- Treat a timeline pose as a baseline: **Edit as a new pose** clones its named
  source, previews bounded joint changes locally, and on save creates a new
  immutable pose assigned only to that clip.
- Use scene clips as a Studio composition aid. Before save or hardware preview,
  Studio recursively flattens them into the existing version-1 semantic events;
  no new persisted scene action is introduced.
- Save a scene as a new YAML file under `scenes/user/`; while connected the
  Pi copy is authoritative, while offline the desktop checkout is staging.
- Connect to the Pi, read lifecycle and terminal results, run named
  scenes/motions/poses, and cancel the active run by its run ID.
- Load the Pi's user-scene library on connection, create new Pi scenes, and
  revision-update an existing Pi user scene without losing concurrent edits.
- Ask `oriond` to reload its validated catalog after every Pi write, then run
  the named pose, motion, or scene on hardware without restarting the daemon.
- Preview the current unsaved scene on connected hardware from the Preview
  dropdown. The gateway size-caps the temporary document, and `oriond`
  validates its named assets before running it without writing a scene file.

Built-in scenes, poses, and motions are source material and are never
overwritten. New user assets use create-only semantics and live under
`scenes/user/`, `motion/user/poses/`, and `motion/motions/user/`. Studio loads
validated local files at desktop startup, then merges the Pi's authoritative
user libraries when it connects. A Pi asset wins over an offline staging copy
with the same user-asset name; no user asset may shadow a built-in.

An edited scene remains a draft. **Save As** always creates a distinct user
scene: directly on the Pi when connected, or in the desktop checkout while
offline. A scene loaded from the Pi also offers **Save changes**. That operation
includes the file revision Studio loaded; a stale save is rejected and must be
reloaded instead of silently replacing newer work. Every accepted create or
update asks `oriond` to reload and validate the complete catalog. A failed
create is removed, and a failed update restores the previous file before the
old catalog is reloaded.

The authenticated gateway API keeps the raw Unix socket private and exposes
only named semantic libraries and operations. It accepts create-new pose and
motion documents, plus create and revision-checked update operations for user
scenes. Its `preview_scene` operation accepts one version-1 semantic scene of
at most 3,000 UTF-8 bytes and never persists it. It never accepts arbitrary
paths, servo registers, or joint streams.

## Connect Studio to the Pi

The Pi runs `oriond.service` and `orion-studio-gateway.service`; both execute
directly from the source checkout and start on reboot. A successful
workstation-side `scripts/deploy_pi.sh` run installs/enables both units,
performs the bounded physical smoke test, and creates a development pairing
token only when one does not already exist. Save the token printed by that
first deployment.

For manual recovery, create a development pairing token once:

```bash
cd /home/mofe/dev/orion
python3 orion_studio/gateway.py create-token \
  --token-file /home/mofe/.config/orion/studio-token
```

The token is printed once and stored with mode `0600`. Start the deliberate
network adapter on the trusted development LAN:

```bash
cd /home/mofe/dev/orion
python3 orion_studio/gateway.py serve \
  --bind 0.0.0.0 \
  --port 7447 \
  --token-file /home/mofe/.config/orion/studio-token
```

In Studio, choose **Connect robot**, use `http://orion.local:7447` (or the Pi's
LAN IP), and paste the token. The gateway URL persists locally; the token lives
only in session storage. The connection status displays the running Rust
binary's embedded Git revision, so a source update is not mistaken for a
completed daemon restart. The raw `/tmp/oriond.sock` socket never leaves the
Pi.

After reboot, Orion intentionally starts torque-off. Running a pose, motion,
or movement-containing scene asks the gateway to prepare movement through the
validated runtime before submitting the named capability. Lighting/audio-only
scenes leave torque off. Studio exposes **Release torque** when Orion is
holding and no run is active.

The gateway has explicit development origins for the local Vite server and
Tauri desktop shells. Add another exact origin only when needed:

```bash
python3 orion_studio/gateway.py serve \
  --bind 0.0.0.0 \
  --token-file /home/mofe/.config/orion/studio-token \
  --allow-origin http://localhost:9000
```

This bearer-token HTTP setup is for a trusted private development network.
Production pairing, encrypted transport, and certificate identity remain later
security work.

## Gateway tests

The adapter tests use a fake local daemon and require no robot hardware:

```bash
python3 -m unittest discover -s orion_studio/tests -v
```

The platform-neutral native scene store can be tested without GTK/WebKit:

```bash
cargo test --manifest-path orion_studio/src-tauri/scene-store/Cargo.toml
```

The Pi wake detector and transcription path remain untouched as a diagnostic
and offline fallback. Future desktop voice work will use a platform-neutral
transcription boundary rather than making Studio's core macOS-specific.
