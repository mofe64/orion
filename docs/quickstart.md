# Orion quickstart

Orion requires a Raspberry Pi 5 with Raspberry Pi OS Lite 64-bit Bookworm or Trixie. Studio runs on your computer.

## Prepare the Pi

1. Select the operating system in Raspberry Pi Imager.
2. Configure the hostname, login user, SSH public key, Wi-Fi and country.
3. Write the image to the boot drive.
4. Boot the Pi.
5. Connect through SSH with your configured user and hostname.

   ```bash
   ssh USER@HOST.local
   ```

## Install Orion

The login user needs sudo access. Models share storage across releases. Allow space for models, the active release and one previous release.

1. Run the installer as the login user.

   ```bash
   curl -fsSL https://raw.githubusercontent.com/mofe64/orion/main/scripts/install.sh | bash
   ```

2. Reboot for kernel changes and device permissions.

   ```bash
   sudo reboot
   ```

3. Reconnect through SSH.
4. Sign in to Codex.

   ```bash
   codex login --device-auth
   ```

5. Complete [servo setup](hardware-versions.md#calibrate-v2-first).

The installer enables the services. First installation leaves them stopped for servo setup. Orion rises slowly to `home` when the services start.

The AI HAT+ is optional. HAILO8L runs face, object and palm networks when available. Each detector can fall back to the CPU. Lamps without the HAT use CPU detectors. See [vision setup](vision-reference.md) for accelerator selection and backend names.

If Hailo identification needs a reboot, run the same installer after reboot to fetch its model files.

## Connect Studio

1. Follow [Studio development](../orion_studio/README.md#development) on your computer.
2. Select **Pair Orion**.
3. Enter the Pi address, such as `http://HOST.local:7447`.
4. Request a pairing code.
5. Enter the code spoken by Orion.
6. Select **Pair**.

**Use a token instead** accepts the Pi's `~/.config/orion/studio-token` file. See [gateway configuration](configuration.md#studio-gateway).

## Update or restore a release

1. Install a selected release with its tag.

   ```bash
   curl -fsSL https://raw.githubusercontent.com/mofe64/orion/main/scripts/install.sh | bash -s -- --version vX.Y.Z
   ```

2. Run the installation checks.

   ```bash
   curl -fsSL https://raw.githubusercontent.com/mofe64/orion/main/scripts/install.sh | bash -s -- --check
   ```

The check prints servo readings, service status and the Hailo result. A failed check returns an error.

CAUTION: Software rollback cannot reverse servo EEPROM zeroing. Use a release with the same V2 rest coordinates after zeroing.

1. Restore the previous release when an update needs reversal.

   ```bash
   curl -fsSL https://raw.githubusercontent.com/mofe64/orion/main/scripts/install.sh | bash -s -- --rollback
   ```

Rollback restores service configuration and package versions. It cannot reverse servo EEPROM changes. The installer refuses rollback into the old V2 joint coordinates.

## Update built-in assets

An asset pack contains built-in poses, motions, scenes and sound cues. User folders remain unchanged. Full app installation also fetches the latest compatible pack.

1. Install the latest pack compatible with the active app.

   ```bash
   curl -fsSL https://raw.githubusercontent.com/mofe64/orion/main/scripts/install.sh | bash -s -- --assets
   ```

2. Select a particular compatible pack when required.

   ```bash
   curl -fsSL https://raw.githubusercontent.com/mofe64/orion/main/scripts/install.sh | bash -s -- --assets --version assets-vX.Y.Z
   ```

The installer checks the pack checksum, asset format and joint-profile checksum before it changes the catalog. It restarts `oriond` only when it was running. Use `--rollback` to restore the previous catalog.

## Logs and local builds

1. Read recent service messages on the Pi.

   ```bash
   journalctl -u oriond -u orion-voice-stack -u orion-vision -n 60 --no-pager
   ```

See [release packaging](release-packaging.md) for local archives and SSH deployment. See [networking](networking.md) for the optional fallback hotspot.
