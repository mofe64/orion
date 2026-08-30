# Orion audio hardware

Orion's current audio HAT is the Seeed Studio ReSpeaker 2-Mics Pi HAT V2,
identified electrically by its TLV320AIC3104 codec at I2C address `0x18`. The
HAT provides two microphones plus playback through its 3.5 mm jack and JST 2.0
speaker output.

This distinction is important: the ReSpeaker V2 overlay targets a
TLV320AIC3104 codec at I2C address `0x18`; the WM8960 is the V1 path at address
`0x1a`. The two overlays are not interchangeable.

The product listing described a WM8960, but Orion's boot diagnostics are the
source of truth: the board acknowledged `0x18`, while a WM8960 probe at `0x1a`
failed with I/O error `-121`. The V1 and V2 overlays are not interchangeable.

Orion uses Seeed's V2 device-tree overlay with the Raspberry Pi kernel's
`snd_soc_tlv320aic3x`, `snd_soc_tlv320aic3x_i2c`, and
`snd_soc_simple_card` modules. The overlay selects the Pi 5 I2S
clock-consumer block and registers the stable ALSA card name
`seeed2micvoicec`. No custom audio kernel module is installed.

## GPIO integration

The audio path uses I2C and I2S. The HAT also exposes BCM12 and BCM13 on its
Grove digital connector, but the audio overlay does not claim them. Orion
already owns BCM12 for the 40-pixel NeoPixel shield, so nothing may be attached
to the ReSpeaker Grove digital connector while that lighting wiring is in use.

The HAT's three APA102 LEDs and user button are outside the first audio slice.
Orion continues to use the 40-pixel RGBW shield as its expressive-light device.

## Persistent Raspberry Pi 5 setup

Install the build and ALSA diagnostic tools:

```bash
sudo apt install device-tree-compiler make alsa-utils i2c-tools
```

Clone Seeed's maintained overlay repository outside Orion, then compile only
the TLV320AIC3104 V2 overlay as the normal development user:

```bash
cd ~/dev
git clone https://github.com/Seeed-Studio/seeed-linux-dtoverlays.git
cd ~/dev/seeed-linux-dtoverlays
make overlays/rpi/respeaker-2mic-v2_0-overlay.dtbo
```

Install Orion's persistent boot configuration:

```bash
cd ~/dev/orion
sudo hardware/audio/install-persistent.sh \
  /home/mofe/dev/seeed-linux-dtoverlays
sudo reboot
```

The installer requires Seeed's V2 overlay targeting the Pi 5 I2S
clock-consumer and TLV320AIC3104 at `0x18`. It installs the compiled overlay,
rejects known conflicting audio overlays, migrates Orion's previous V1 boot
entry to one idempotent `dtoverlay=respeaker-2mic-v2_0` entry, and preserves
the original boot configuration as `config.txt.orion-audio-backup`. It does
not install custom kernel modules.

After reboot, verify the codec, playback, capture, and NeoPixel integration:

```bash
cd ~/dev/orion
hardware/audio/verify-persistent.sh
```

The expected ALSA card name is `seeed2micvoicec`. The verifier also requires
the TLV320AIC3104 at I2C address `0x18` to be bound to its kernel driver. When
Orion's NeoPixel device is present, it confirms BCM12 remains assigned to
PWM0.

## Mixer commissioning

Do not install a guessed global ALSA state. First inspect the controls exposed
by the running kernel:

```bash
amixer -c seeed2micvoicec scontrols
amixer -c seeed2micvoicec contents
```

The first physical playback check will use the stable ALSA name rather than a
numeric card index. Start with a low mixer level, then play one left-channel
test tone through the HAT:

```bash
alsamixer -c seeed2micvoicec
speaker-test \
  -D plughw:CARD=seeed2micvoicec,DEV=0 \
  -c 2 -s 1 -t sine -f 440 -l 1
```

Once the speaker path is confirmed, Orion will capture the minimal working
TLV320AIC3104 mixer state in this directory and add an automated playback
verifier. The runtime WAV-cue backend comes after this hardware contract is
commissioned.
