# Orion V1 audio hardware

V1 uses the ReSpeaker 2-Mics Pi HAT V2. A HAT is an expansion board mounted on the Pi header. V2 uses USB audio; see [V2 audio](../../docs/hardware-versions.md#usb-capture-and-playback).

The V1 board uses a TLV320AIC3104 codec at I2C address `0x18`. Its ALSA card name is `seeed2micvoicec`. ALSA is Linux's audio interface.

## Install the V1 overlay

The overlay uses Raspberry Pi kernel audio modules. It does not install a custom audio kernel module. Run the Orion commands from the repository root.

CAUTION: The Grove digital connector exposes lighting pins. Leave it disconnected while Orion uses GPIO12 for lighting.

1. Install overlay build tools.

   ```bash
   sudo apt-get install -y device-tree-compiler make alsa-utils i2c-tools git
   ```

2. Clone Seeed's overlay sources outside Orion.

   ```bash
   git clone https://github.com/Seeed-Studio/seeed-linux-dtoverlays.git "$HOME/seeed-linux-dtoverlays"
   ```

3. Build the V2 HAT overlay.

   ```bash
   make -C "$HOME/seeed-linux-dtoverlays" overlays/rpi/respeaker-2mic-v2_0-overlay.dtbo
   ```

4. Install persistent audio configuration.

   ```bash
   sudo hardware/audio/install-persistent.sh "$HOME/seeed-linux-dtoverlays"
   ```

5. Reboot the Pi.

   ```bash
   sudo reboot
   ```

6. Check the audio device and pin configuration.

   ```bash
   hardware/audio/verify-persistent.sh
   ```

The installer preserves the original boot configuration as `config.txt.orion-audio-backup`.

## Playback and capture

1. Apply the JST speaker route.

   ```bash
   hardware/audio/configure-playback.sh
   ```

WARNING: A test tone can be loud. Keep the speaker away from your ears.

2. Play a right-channel test tone.

   ```bash
   speaker-test -D plughw:CARD=seeed2micvoicec,DEV=0 -c 2 -s 2 -t sine -f 440 -l 1
   ```

3. Apply the microphone capture route.

   ```bash
   hardware/audio/configure-capture.sh
   ```

Playback uses the right differential output for the JST amplifier. Capture selects the two microphone inputs with automatic gain control disabled. Listener gain settings are in [configuration](../../docs/configuration.md#microphone-listener).

## Desktop audio management

WirePlumber can change the HAT mixer while Orion runs. The supplied rule excludes this device from WirePlumber 0.5.

1. Install the device exclusion rule when using a Pi desktop.

   ```bash
   sudo install -D -m 0644 hardware/audio/90-orion-respeaker.conf /etc/wireplumber/wireplumber.conf.d/90-orion-respeaker.conf
   ```

2. Restart the user's audio manager.

   ```bash
   systemctl --user restart wireplumber
   ```

3. Reapply the speaker route.

   ```bash
   hardware/audio/configure-playback.sh
   ```
