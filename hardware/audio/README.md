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

## Mixer commissioning

Orion keeps the confirmed JST-speaker mixer route as a repeatable command
rather than depending on whatever mixer state happened to survive the last
session:

```bash
hardware/audio/configure-playback.sh
```

The script selects `DAC_R1`, sends it through the right line mixer, keeps both
analogue stages at unity gain, and sets PCM to the commissioned
physical-acceptance target of `0 dB`.
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
opened, so source-run development does not depend on a system boot service or
a globally stored ALSA snapshot.

Orion also keeps the confirmed dual-microphone capture route as a repeatable
command:

```bash
hardware/audio/configure-capture.sh
```

The script selects the HAT's single-ended `LINE1L` and `LINE1R` microphone
routes, disables the codec's automatic gain control (AGC), and applies a fixed
50 dB programmable-gain amplifier (PGA) capture gain. The wake worker runs this
script automatically before opening `arecord`; direct recording tests can run
it explicitly. This prevents wake-word behavior from depending on whatever
capture level a previous process left in the codec. Physical commissioning
found that 50 dB recognized the wake
phrase reliably, while the codec's 59.5 dB maximum degraded detection through
excess noise or clipping. Codec AGC remains disabled.

## Commissioning result

The assembled Pi 5 passed the persistent V2 verification with playback and
capture registered as `seeed2micvoicec`, while BCM12 remained assigned to the
NeoPixel pulse-width modulation (PWM) output. The JST route produced the 440 Hz
right-channel test tone,
the direct cue command played Orion's local chime, and both expressive
acknowledgement scenes exercised that ReSpeaker playback path successfully.
The commissioned acceptance target is `0 dB`; final listening acceptance must
confirm that speech is clear without audible clipping at the assembled JST
speaker.

## Planned audio-front-end upgrade

The commissioned Pi listener consumes 16 kHz mono audio even though the mixer
enables both microphone routes. Stereo capture, channel diagnostics, adaptive
combination, denoising, beamforming, and playback-reference echo cancellation
are not implemented. See the
[audio-front-end upgrade](../../docs/explanation/audio-front-end-upgrade.md)
for its boundary, phases, expected limits, and validation gate.
