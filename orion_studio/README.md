# Orion Studio

Orion Studio is the cross-platform desktop workspace for previewing and
authoring Orion scenes, then submitting named work to the robot.

```text
Tauri + React Studio                    Raspberry Pi
┌─────────────────────────┐  HTTP v1   ┌──────────────────────────┐
│ URDF preview + timeline │───────────▶│ gateway.py               │
│ local YAML asset catalog│  token     │ semantic allowlist       │
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

The production bundle is configured for macOS, Windows, and Linux. Native
packaging still needs to be built and signed on each target OS.

During source development, Studio resolves the Orion project from its Tauri
crate location. A packaged or relocated build can point at a checkout explicitly:

```bash
ORION_PROJECT_ROOT=/path/to/orion pnpm tauri dev
```

## What Studio can do now

- Load Orion's real URDF and STL meshes into an orbitable 3D preview.
- Browse the existing scenes, named poses, and authored motions directly from
  the Rust/YAML project assets.
- Preview pose and motion timing locally with the same quintic blend shape used
  by the runtime.
- Scrub and play a multi-lane scene timeline for movement, RGBW lighting, and
  queued local audio cues.
- Add, select, drag to retime, edit, and delete scene events, and edit the
  scene description.
- Save a scene as a new YAML file under `scenes/user/`.
- Connect to the Pi, read lifecycle and terminal results, run named
  scenes/motions/poses, and cancel the active run by its run ID.
- Publish a saved user scene to the Pi, ask `oriond` to reload its validated
  catalog, and run it on hardware without restarting the daemon.

Built-in scenes and poses are source material and are never overwritten. The
native save command validates the scene and uses create-new file semantics, so
an existing user scene is not overwritten either. See `scenes/user/README.md`.
Studio reloads validated files from `scenes/user/` into the Library when the
desktop app starts, and a newly saved scene appears there immediately.

An edited scene remains a draft until **Save As** creates its local user copy.
For a clean user scene the hardware action becomes **Publish & Run**. Publishing
creates `scenes/user/<name>.yaml` on the Pi, never replaces different content,
asks `oriond` to reload and validate the complete catalog, then submits the
scene by name. A failed reload removes the newly published file.

## Connect Studio to the Pi

Continue running `oriond` from the Pi source checkout; do not install it as a
systemd service. A successful workstation-side `scripts/deploy_pi.sh` run
starts both the source-built daemon and gateway, performs the bounded physical
smoke test, and creates a development pairing token only when one does not
already exist. Save the token printed by that first deployment.

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
