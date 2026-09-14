# Orion quickstart

Run these commands from the repository root. Studio voice processing requires
an Apple Silicon Mac, an installed Codex runtime with a working login, and a Pi
with the runtime, gateway, and listener installed.

## Start Studio with its UI

Install Node.js 20 or newer, pnpm, Rust, uv, and the
[Tauri prerequisites](../orion_studio/README.md#development), then prepare the
frontend and speech models:

```bash
pnpm --dir orion_studio install
uv sync --project speech --python 3.12 --locked
speech/.venv/bin/orion-voice-models
scripts/studio-dev.sh
```

Select **Pair Orion**, enter the gateway address and token, and save the pairing.
Choose speech and agent settings in Settings. The UI can own voice processing
until headless mode is installed; in that mode, quitting the app stops processing.

## Install headless mode

Quit Studio once to release its voice owner, then run on the Mac:

```bash
scripts/studio-headless.sh install
scripts/studio-headless.sh status
```

Installation builds and tests a release, starts it in the background, and registers
it with macOS launchd. It starts again after you log in and restarts if it crashes.
Keep the Mac awake and connected to the Pi. This user service starts after login,
so a reboot still requires a login before voice processing resumes.

Open `scripts/studio-dev.sh` whenever you want the UI. It attaches to the background
service, and quitting the UI leaves voice processing running. Settings, pairing,
memories, and personality edits go to that same owner.

## Manage the background service

| Action | Command |
| --- | --- |
| Show process and coordinator status | `scripts/studio-headless.sh status` |
| Follow logs | `scripts/studio-headless.sh logs` |
| Stop until the next start or login | `scripts/studio-headless.sh stop` |
| Start an installed service | `scripts/studio-headless.sh start` |
| Restart it | `scripts/studio-headless.sh restart` |
| Remove automatic startup | `scripts/studio-headless.sh uninstall` |

A running control service can still be waiting for pairing, models, or the Pi.
Check `error` in status and use Studio Debug for voice readiness. Stopping the
service cancels its playback and ends its conversation. Pi microphone capture
continues according to its saved listening preference. Uninstall retains releases,
models, pairing, and personal data, and permits the UI to own processing again.

## Update headless mode

Build from the files in your checkout, including uncommitted source changes:

```bash
scripts/update-studio-headless.sh
```

To fetch and fast-forward the checkout's upstream branch first, commit your work
and run:

```bash
scripts/update-studio-headless.sh --pull
```

The updater snapshots the service, coordinator, agent, and speech packages. It
installs locked Python dependencies, runs their tests, and builds the release
binary before stopping the active service. It then switches releases and starts
through launchd. Failed preparation leaves the old service running; failed
activation restores the previous release and its prior running state.

Updates preserve saved settings, pairing, downloaded models, memories, and
personality. Restarting starts a fresh agent conversation. If the UI was open
during the restart, reopen it to attach to the new voice observer. The updater retains
older releases for recovery, so release directories can accumulate over time.
See [configuration](configuration.md) for paths and environment overrides.

## Deploy to the Pi

Push the intended branch to the Pi repository's remote, then run:

```bash
scripts/deploy_pi.sh --host mofe@orion.local \
  --root /home/mofe/dev/orion --branch main
```

This command updates the Pi runtime, gateway, and listener and enables their
systemd services. It performs physical movement, light, and audio checks; follow
[servo commissioning](../hardware/servo_setup/README.md) and supervise the robot.
Use your Pi account, checkout path, and branch if they differ. See the
[deployment procedure](../runtime/README.md#deploy-an-update-to-the-raspberry-pi)
for first setup and recovery.

The Mac and Pi have separate deployments. Run the headless updater as well when
a change affects both sides. To check Pi services over SSH:

```bash
ssh mofe@orion.local \
  'systemctl status oriond.service orion-studio-gateway.service orion-listener.service'
```
