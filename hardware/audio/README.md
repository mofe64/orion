# Orion audio hardware

Orion's current audio HAT is the Seeed Studio ReSpeaker 2-Mics Pi HAT V1,
identified by its WM8960 codec. The HAT provides two microphones plus playback
through its 3.5 mm jack and JST 2.0 speaker output.

This distinction is important: the newer ReSpeaker V2 overlay targets a
TLV320AIC3104 codec at I2C address `0x18`. Orion's WM8960 is the V1 path at
address `0x1a`; the two overlays are not interchangeable.

The old `respeaker/seeed-voicecard` package builds out-of-tree kernel modules
and does not list Raspberry Pi 5 as a supported platform. Orion instead uses
the current Seeed device-tree overlay with the Raspberry Pi kernel's built-in
`snd_soc_wm8960` and `snd_soc_simple_card` modules. The current V1 overlay
explicitly supports `brcm,bcm2712`, selects the Pi 5 I2S clock-consumer block,
and registers the stable ALSA card name `seeed2micvoicec`.

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
the WM8960 V1 overlay as the normal development user:

```bash
cd ~/dev
git clone https://github.com/Seeed-Studio/seeed-linux-dtoverlays.git
cd ~/dev/seeed-linux-dtoverlays
make overlays/rpi/respeaker-2mic-v1_0-overlay.dtbo
```

Install Orion's persistent boot configuration:

```bash
cd ~/dev/orion
sudo hardware/audio/install-persistent.sh \
  /home/mofe/dev/seeed-linux-dtoverlays
sudo reboot
```

The installer requires a current Seeed overlay that declares Pi 5 and WM8960
compatibility. It installs the compiled overlay, rejects known conflicting
audio overlays, adds one idempotent `dtoverlay=respeaker-2mic-v1_0` entry, and
preserves the current boot configuration as `config.txt.orion-audio-backup`
before changing it. It does not install custom kernel modules.

After reboot, verify the codec, playback, capture, and NeoPixel integration:

```bash
cd ~/dev/orion
hardware/audio/verify-persistent.sh
```

The expected ALSA card name is `seeed2micvoicec`. The verifier also requires
the WM8960 at I2C address `0x1a` to be bound to its kernel driver. When Orion's
NeoPixel device is present, it confirms BCM12 remains assigned to PWM0.

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
WM8960 mixer state in this directory and add an automated playback verifier.
The runtime WAV-cue backend comes after this hardware contract is commissioned.
