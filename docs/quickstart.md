# Orion quickstart

Run repository commands from the Orion source root. Voice and agent processing
run on the Pi; Studio is an optional desktop controller.

## Start Studio with its UI

Install Node.js 20 or newer, pnpm, Rust, and the
[Tauri prerequisites](../orion_studio/README.md#development):

```bash
pnpm --dir orion_studio install
scripts/studio-dev.sh
```

Select **Pair Orion**, enter the gateway address and token, and save the pairing.
Settings and profile changes are sent to the Pi. Closing Studio leaves Orion
running. Studio requires no speech models or background service on the Mac.
An unavailable Pi produces a connection error; Studio never starts local inference.

## Deploy to the Pi

Push the intended branch to the Pi repository's remote, then run:

```bash
scripts/deploy_pi.sh --host mofe@orion.local \
  --root /home/mofe/dev/orion --branch main
```

This command builds a complete release of the runtime, gateway, listener, and
onboard voice/agent service before stopping the running services. It fetches the
selected commit into a separate release directory; the Pi checkout and its local
edits stay intact. Use your Pi account, checkout path, and branch if they differ.
The switch returns Orion to rest and releases torque; normal character startup
may move it home. Keep the robot clear during the switch.

The deployment preserves saved voice and agent preferences, Codex login,
pairing, microphone settings, calibration, personality, memory, and the existing
motion/user-asset catalog. It repoints service executables and the speech Python
environment, preserving other environment values and command arguments, including
listener tuning and the 30-minute sleep override. Existing service enablement is
retained; missing services are enabled.

All four services must start, the runtime revision must match, Qwen/Pocket/Codex
must report ready, and the gateway must reach that same voice service. A failed
switch restores the configuration and service state captured immediately before
that update. The last successful update retains one rollback snapshot. Restarting
the agent service begins a fresh conversation; saved memory remains.

## Pi local voice and agent

The Pi 5 (8 GB, 64-bit Linux) runs Rustpotter, Silero, Qwen3-ASR, Pocket TTS,
and the Rust coordinator without Studio. Codex App Server runs locally and uses
your ChatGPT subscription online.

The Pi needs working hardware/audio setup, servo calibration and a pairing token
under `~/.config/orion/`, plus Git, Rust/Cargo, uv, Python 3.11 or later, and the
native build/audio dependencies (`build-essential`, `pkg-config`, `libssl-dev`,
`libdbus-1-dev`, `alsa-utils`, `python3-venv`, `ca-certificates`). The deployment
preserves hardware and WirePlumber configuration. It does not set up or recalibrate
the hardware.

For the first onboard installation, commit the intended source and prepare a
complete release on the Pi without activating it:

```bash
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
python3 scripts/deploy_pi_release.py --source "$PWD" --revision HEAD --prepare-only
```

The command prints the permanent release path. It builds and tests the runtime,
trajectory compiler, gateway, listener and voice host. The preparer downloads
Qwen GGUF, native llama-server libraries, Codex, and Silero from upstream and
verifies pinned SHA-256 hashes. Pocket 3.1.0 supplies pinned model and preset
revisions. Each release gets its own locked speech/listener environments; shared
downloads and their inventories live under `~/.local/share/orion/voice-stack`.
No experiment directory is required. Matching assets are reused; mismatched
files are rejected without replacement.

Sign in over SSH as the Pi user, then activate the printed release path:

```bash
~/.local/share/orion/voice-stack/codex-0.154.0/bin/codex login --device-auth
~/.local/share/orion/voice-stack/codex-0.154.0/bin/codex login status
python3 /path/to/prepared-release/scripts/install_pi_voice_stack.py --release /path/to/prepared-release
systemctl is-active oriond orion-studio-gateway orion-listener orion-voice-stack
```

Complete the displayed device code in your browser. Do not run Codex login or
the installer with sudo; the installer requests sudo for service control and
system files. Subsequent updates use `scripts/deploy_pi.sh` from the workstation.
The compatibility entry point `scripts/install_pi_services.sh ROOT USER HOME`
also builds and activates a full release from committed `HEAD`. The standalone
`install_pi_voice.sh` refuses to update an installed onboard stack.

Use `--plan` on the release installer to list affected files without changing
services. The installer checks the existing motion catalog before activation;
it does not migrate or replace poses, scenes, audio cues, or user assets.
After deployment, complete a spoken turn to check the physical microphone and
speaker; automated readiness checks do not exercise them.

For logs and rollback, use the installer from the most recently prepared release:

```bash
journalctl -u oriond -u orion-voice-stack -u orion-listener -u orion-studio-gateway -n 60 --no-pager
python3 /path/to/prepared-release/scripts/install_pi_voice_stack.py --rollback
```

Rollback restores the immediately previous installation, including active and
enabled service states. It retains models, Codex login and saved user preferences.
If interruption or recovery failure leaves `pending-installation.json`, another
update is blocked until `--rollback` completes. A failed mechanical rest leaves
the runtime running instead of forcing termination.

`installation.json` identifies the active release and recent transaction backup.
`downloads.json` and `model-files.json` inventory shared assets. Transaction
snapshots are pruned after a successful update; release directories are retained
so rollback never loses its binaries or Python environments. Keep the active
release and the release paths referenced by its rollback snapshot. Remove older
unused releases separately after inspection, and keep the managed Python
interpreters used by retained environments.
