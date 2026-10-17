# Orion quickstart

The Pi runs Orion’s servos, camera, microphone, local speech workers and Codex
coordinator. Studio connects from your computer for pairing and authoring.

## Prepare a new Pi

1. In Raspberry Pi Imager, flash **Raspberry Pi OS Lite 64-bit**, Bookworm or
   Trixie, for a Raspberry Pi 5. Configure the hostname, login user, SSH public
   key, Wi-Fi and country before writing the image.
2. Boot the Pi and connect with `ssh USER@HOST.local`.
3. Install Orion with the command below. The user needs sudo. Models download
   once into `~/.local/share/orion/voice-stack/models/` and are verified before
   reuse. Keep enough space for the active release, one rollback and models.

## Install or update

```bash
curl -fsSL https://raw.githubusercontent.com/mofe64/orion/main/scripts/install.sh | bash
```

The installer accepts Raspberry Pi OS Bookworm/Trixie ARM64. It installs the
system packages, verifies a GitHub release archive and its file manifest,
installs release-supplied Python 3.12 at `/opt/orion/python`, and creates isolated
venvs offline. It never uses the OS Python for Orion components.

For a specific release or a local Docker build:

```bash
curl -fsSL https://raw.githubusercontent.com/mofe64/orion/main/scripts/install.sh | bash -s -- --version vX.Y.Z
bash scripts/install.sh --from artifacts/orion-X.Y.Z-arm64.tar.gz
```

The local tarball needs its adjacent `.sha256` file. Build one with
`scripts/build_release.sh vX.Y.Z`; Docker runs ARM64 Debian Bookworm for shared
binaries and Bookworm/Trixie separately for native camera/Hailo bindings.
See [release packaging](release-packaging.md) for development and verification.

Reboot after the first install for kernel modules and group permissions.
Then sign in as the Pi login user:

```bash
codex login
```

For new servos, assign IDs one at a time; then fold the assembled lamp into
[mechanical rest](hardware-versions.md#calibrate-v2-first) and run:

```bash
oriond servo zero
oriond --check
sudo systemctl start oriond orion-listener orion-studio-gateway orion-voice-stack orion-vision
```

Keep the area clear: Orion rises slowly to home. The installer enables the
units but leaves motion stopped when the direct hardware check fails. The
runtime also checks its joint profile before enabling torque. Updates reuse the
same command and preserve pairing, credentials, preferences and user assets.

## Connect Studio

On your computer, follow [Studio development](../orion_studio/README.md#development)
to install Node.js, pnpm, Rust and the Tauri prerequisites. From the source root:

```bash
pnpm --dir orion_studio install
scripts/studio-dev.sh
```

Choose **Pair Orion**, enter `http://HOST.local:7447`, and request a code. Type
the code spoken by the lamp and choose **Pair**. Studio saves the verified
connection. **Use a token instead** accepts `~/.config/orion/studio-token` from
the Pi. See [pairing details](configuration.md#studio-gateway).

## Deploy to the Pi

Install a published release over SSH with
`scripts/deploy_pi.sh --host USER@HOST --version vX.Y.Z`. For development, build
locally and pass `--from FILE` instead. SSH must already trust the Pi’s host key.
The same installer handles both paths.

The optional [Wi-Fi fallback hotspot](networking.md) works with the
NetworkManager configuration supplied by Raspberry Pi OS Imager.

## Logs and recovery

```bash
systemctl status oriond orion-listener orion-studio-gateway orion-voice-stack orion-vision
journalctl -u oriond -u orion-voice-stack -u orion-vision -n 60 --no-pager
```

Studio’s Debug page reports model and service readiness. A running unit alone
does not prove microphone, camera or inference health.

Restore the immediately previous release with:

```bash
curl -fsSL https://raw.githubusercontent.com/mofe64/orion/main/scripts/install.sh | bash -s -- --rollback
```

Release directories and transaction snapshots stay under
`~/.local/share/orion/voice-stack/`. Rollback restores service files and built-in
assets together; shared models remain available. A failed activation attempts
that restoration automatically. Do not restore a release using the old V2
calibration coordinate frame after writing mechanical-rest servo offsets.

The explicit AI HAT+ check is `orion-check-hailo` on the Pi. It
identifies Hailo-8/8L, opens the device through the vision venv, and runs a pinned
stock model. See [Hailo setup](../vision/README.md#ai-hat-hailo-setup).
