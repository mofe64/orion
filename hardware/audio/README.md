# Orion audio hardware

Orion's installed audio board is the Seeed Studio ReSpeaker 2-Mics Pi HAT V2,
a Raspberry Pi Hardware Attached on Top (HAT). Its TLV320AIC3104 codec uses
Inter-Integrated Circuit (I2C) address `0x18`. The HAT provides two microphones
plus playback through its 3.5 mm jack and JST 2.0 speaker output.

Orion uses Seeed's V2 device-tree overlay with the Raspberry Pi kernel's
`snd_soc_tlv320aic3x`, `snd_soc_tlv320aic3x_i2c`, and
`snd_soc_simple_card` modules. The overlay selects the Pi 5 Inter-IC Sound
(I2S) clock-consumer block and registers the stable Advanced Linux Sound
Architecture (ALSA) card name `seeed2micvoicec`. No custom audio kernel module
is installed.

## General-purpose input/output integration

The audio path uses I2C and I2S. The HAT also exposes BCM12 and BCM13 on its
general-purpose input/output (GPIO) Grove digital connector, but the audio
overlay does not claim them. Orion
already owns BCM12 for the 40-pixel NeoPixel shield, so nothing may be attached
to the ReSpeaker Grove digital connector while that lighting wiring is in use.

The HAT's three APA102 LEDs and user button are not used. Orion uses the
40-pixel red-green-blue-white (RGBW) shield as its expressive-light device.

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
rejects known conflicting audio overlays, migrates Orion's previous boot
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

## Mixer setup

Orion keeps the confirmed JST-speaker mixer route as a repeatable command
rather than depending on whatever mixer state happened to survive the last
session:

```bash
hardware/audio/configure-playback.sh
```

The script selects `DAC_R1`, sends it through the right line mixer, keeps both
analogue stages at unity gain, and sets PCM to the `0 dB` playback target.
The right differential line output feeds the V2 HAT's mono amplifier and JST
connector; the `HP` controls instead serve the 3.5 mm jack.

The physical playback check uses the stable ALSA name rather than a numeric
card index and sends the tone to the right channel:

```bash
speaker-test \
  -D plughw:CARD=seeed2micvoicec,DEV=0 \
  -c 2 -s 2 -t sine -f 440 -l 1
```

The runtime applies the same mixer contract when its physical WAV backend is
opened, so running from a checkout does not depend on a system boot service or
a globally stored ALSA snapshot.

On Pi desktop installations, WirePlumber can probe the same card after Orion
starts and reset PCM to `-23.5 dB`. Install the supplied
`90-orion-respeaker.conf` under `/etc/wireplumber/wireplumber.conf.d/` during audio
setup to exclude this ReSpeaker device from WirePlumber 0.5. The application
release installer preserves that hardware configuration; it does not create it. Orion's direct
ALSA playback and capture remain available; desktop applications no longer
see this card. HDMI audio is unaffected.

To apply this fix to an existing Pi without rebooting, run as the desktop user
from the Orion checkout:

```bash
sudo install -D -m 0644 hardware/audio/90-orion-respeaker.conf \
  /etc/wireplumber/wireplumber.conf.d/90-orion-respeaker.conf
systemctl --user restart wireplumber
hardware/audio/configure-playback.sh
```

Check that `amixer -c seeed2micvoicec sget PCM` reports `0.00dB` on both
channels and that `wpctl status -n` does not list the ReSpeaker device
`alsa_card.platform-soc_107c000000_sound`. The device-name match is specific
to the installed Pi 5 overlay; recheck it when changing boards or overlays.

Orion also keeps the confirmed dual-microphone capture route as a repeatable
command:

```bash
hardware/audio/configure-capture.sh
```

The script selects the HAT's single-ended `LINE1L` and `LINE1R` microphone
routes, disables the codec's automatic gain control (AGC), and applies a fixed
programmable-gain amplifier (PGA) capture gain. `ORION_CAPTURE_GAIN_DB` accepts
the range documented in [listener configuration](../../docs/configuration.md#pi-runtime-and-listener).
The managed listener supplies its own gain override. The listener configures routing before opening `arecord`, discards
300 ms, reapplies the gain after the ADC starts, then discards another 300 ms.
Reapplying gain prevents ADC startup from undoing the chosen setting. Direct
recording tests can run the script explicitly. Earlier wake tests found reliable
detection at 50 dB but degraded detection at the codec's 59.5 dB maximum through
noise or clipping. The managed voice stack uses the lower configured gain after
further speech trials. Codec AGC remains disabled.

## Hardware checks

The assembled Pi 5 passed the persistent V2 verification with playback and
capture registered as `seeed2micvoicec`, while BCM12 remained assigned to the
NeoPixel pulse-width modulation (PWM) output. The JST route produced the 440 Hz
right-channel test tone,
the direct cue command played Orion's local chime, and both expressive
acknowledgement scenes exercised that ReSpeaker playback path successfully.
The playback target is `0 dB`; listening checks must
confirm that speech is clear without audible clipping at the assembled JST
speaker.

## Stereo voice capture

The Pi Rustpotter listener requests synchronized stereo PCM16 at 16 kHz,
retains stereo for coarse direction estimates, and downmixes to mono for wake
and ASR. Stereo channel independence, orientation and direction accuracy still
require physical checks; see [Pi voice setup](../../voice/README.md).
