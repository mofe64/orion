# Orion quickstart

Orion runs on a Raspberry Pi 5 (required because the speech server uses CPU instructions specific to the Pi 5) with Raspberry Pi OS Lite 64-bit (Bookworm or Trixie). The Studio desktop app runs on your computer.

## Prepare the Pi

1. Select **Raspberry Pi OS Lite (64-bit)** in Raspberry Pi Imager.
2. In Imager's OS customisation settings, configure a hostname, login user, SSH public key, Wi-Fi network and country.
3. Write the image to your microSD card or NVMe drive and boot the Pi.
4. Connect over SSH from your computer.

   ```bash
   ssh USER@HOST.local
   ```

## Install Orion

The login user needs `sudo` access. Shared AI models stay outside individual releases, so leave room for the models, the active release and one previous release for rollback.

1. Run the installer as the login user.

   ```bash
   curl -fsSL https://raw.githubusercontent.com/mofe64/orion/main/scripts/install.sh | bash
   ```

2. Reboot so kernel changes and device permissions take effect.

   ```bash
   sudo reboot
   ```

3. Reconnect over SSH and sign in to Codex (used for Orion's replies and web search).

   ```bash
   codex login --device-auth
   ```

4. Complete [servo setup](hardware-versions.md#calibrate-v2-first), which assigns each servo ID, sets the mechanical rest zero, and starts the Orion services.
5. Clear the space around the lamp as it rises to `home`, then say **"Hey Orion, what time is it?"** to confirm it hears you, speaks back and moves.

First-time installation enables the systemd services but leaves them stopped until servo setup finishes.

The Raspberry Pi AI HAT+ is optional. When a HAILO8L HAT is present, face, object and palm detection run on the accelerator with automatic CPU fallback; lamps without a HAT run all detectors on the CPU. If the Pi needs a reboot before it detects the HAT, run the same installer command once more after rebooting to download the Hailo model files. See [vision detectors](vision-reference.md) for details.

## Connect Studio

Studio is currently built and launched from source on your computer rather than shipped as a standalone installer.

1. Follow the [Studio development setup](../orion_studio/README.md#development) on your computer to launch the app.
2. Select **Pair Orion**.
3. Enter the Pi gateway address, such as `http://HOST.local:7447`.
4. Request a pairing code; Orion speaks a six-digit code aloud.
5. Enter the code and select **Pair**.

If Orion cannot speak the code, select **Use a token instead** and paste the contents of `~/.config/orion/studio-token` from the Pi (see [gateway configuration](configuration.md#studio-gateway)).

## Update or restore a release

1. Install a specific release by tag.

   ```bash
   curl -fsSL https://raw.githubusercontent.com/mofe64/orion/main/scripts/install.sh | bash -s -- --version vX.Y.Z
   ```

2. Run the installation check to print servo readings, service status and Hailo status (exits non-zero on failure).

   ```bash
   curl -fsSL https://raw.githubusercontent.com/mofe64/orion/main/scripts/install.sh | bash -s -- --check
   ```

CAUTION: Software rollback cannot undo servo EEPROM zeroing. Only roll back to a release that uses the same V2 mechanical-rest coordinates; the installer blocks rolling back into older pre-rest-frame releases.

3. Restore the previous release if an update needs to be reversed.

   ```bash
   curl -fsSL https://raw.githubusercontent.com/mofe64/orion/main/scripts/install.sh | bash -s -- --rollback
   ```

## Update built-in assets

Asset packs update built-in poses, motions, scenes and sound cues without touching your user-authored files. Full app releases already bundle and fetch the latest compatible pack automatically.

1. Install the latest asset pack compatible with your active Orion release.

   ```bash
   curl -fsSL https://raw.githubusercontent.com/mofe64/orion/main/scripts/install.sh | bash -s -- --assets
   ```

2. Install a specific asset pack version when needed.

   ```bash
   curl -fsSL https://raw.githubusercontent.com/mofe64/orion/main/scripts/install.sh | bash -s -- --assets --version assets-vX.Y.Z
   ```

The installer verifies the pack checksum, asset format and joint-profile checksum before switching, and restarts `oriond` only if it was already running. `--rollback` restores the previous asset catalogue.

## Logs and local builds

Service logs persist across reboots and power loss and are capped at 200 MB.

1. View recent messages from the core services on the Pi.

   ```bash
   journalctl -u oriond -u orion-voice-stack -u orion-vision -n 60 --no-pager
   ```

For building local release archives or deploying over SSH, see [release packaging](release-packaging.md). For setting up an offline Wi-Fi recovery hotspot, see [Wi-Fi fallback](networking.md).

## Home Assistant

Orion can run Home Assistant Container locally on the Pi (Raspberry Pi OS ARM64) so you can control household lights, switches, fans, climate devices, covers and media players. Install Orion first.

1. Open **Home** in Studio and select **Install Home Assistant**.
2. Wait for Home Assistant to start, then complete **Set up Home** with your name, username and password.
3. Select **Add** beside a discovered device, or select **Add device** to search by brand.
4. Turn on **Voice control** for each device you want Orion to control by voice.

Studio installs Docker and starts the `orion-home-assistant` container without requiring a reboot or Docker group changes. You can also run the same installation from the Pi terminal:

```bash
curl -fsSL https://raw.githubusercontent.com/mofe64/orion/main/scripts/install.sh | bash -s -- --home-assistant
```

Orion updates and rollbacks leave the Home Assistant container untouched. Update it explicitly to the image pinned by your installed release when needed:

```bash
curl -fsSL https://raw.githubusercontent.com/mofe64/orion/main/scripts/install.sh | bash -s -- --home-assistant --update
```

Home Assistant listens on port `8123` (**Open Home Assistant** in Studio opens it in your browser). USB Zigbee and Thread radios require manual Docker `--device` mappings and are not configured automatically; see [Home Assistant configuration](configuration.md#home-assistant) for details.
