# Orion lighting hardware

Orion's installed light is an Adafruit NeoPixel Shield, product 2864:

- 40 individually addressable SK6812-compatible RGBW pixels.
- Physical 5 by 8 matrix.
- Dedicated approximately 3000 K warm-white channel.
- 8-bit red, green, blue, and white channels.
- 800 kHz single-wire NeoPixel protocol.

The assembled robot reports this direct Raspberry Pi 5 wiring:

```text
Pi physical pin 4  / 5 V   -> shield 5 V
Pi physical pin 30 / ground -> shield ground
Pi physical pin 32 / BCM12 -> shield D6 / DIN
```

There is currently no 3.3 V-to-5 V data level shifter. The physical backend
must therefore use BCM12, PWM channel 0, non-inverted output, 40 pixels, and an
initial GRBW/800 kHz configuration.

Physical commissioning on Orion confirmed a non-serpentine, row-major 8 by 5
matrix. Every row runs left to right, starting at the top:

```text
 0  1  2  3  4  5  6  7
 8  9 10 11 12 13 14 15
16 17 18 19 20 21 22 23
24 25 26 27 28 29 30 31
32 33 34 35 36 37 38 39
```

Therefore a zero-based `(row, column)` coordinate maps directly to
`row * ORION_LIGHT_WIDTH + column`. Confirmed reference points are pixel 0 at
top-left, 7 at top-right, 8 at the second row's left edge, and 39 at
bottom-right.

Commissioning also confirmed that Orion's logical channel arguments display as
red, green, blue, and warm white respectively. The backend's GRBW wire-order
translation is therefore correct for the installed shield.

The complete 40-pixel frame was verified on the physical robot using the
acknowledgement value `RGBW(8, 3, 0, 20)`, followed by a successful all-off
frame. The persistent module, overlay, PWM channel, and BCM12 pin service were
then verified after a full robot reboot with `verify-persistent.sh`. Physical
lighting output and its boot configuration are commissioned.

The shield is powered from the Pi's 5 V header rather than an independent
supply. The runtime does not impose a brightness ceiling: RGBW values are sent
exactly as requested. Use low values while confirming this particular wiring
and channel order:

```text
all off
pixel 0 red
pixel 0 green
pixel 0 blue
pixel 0 white
low-output 40-pixel chase
```

This sequence establishes channel order, data reliability, pixel count, and
matrix orientation. If the direct 3.3 V data signal produces flicker,
incorrect colours, or intermittent updates, the hardware path requires a
5 V-compatible logic-level shifter; scene or colour code should not compensate
for signalling errors.

## Persistent Raspberry Pi 5 setup

Raspberry Pi 5 support in `rpi_ws281x` uses the RP1 PWM kernel module from its
`pi5` branch. Orion keeps the upstream source outside this repository and owns
the boot installation through `install-persistent.sh`.

Install the exact headers for the running kernel and the remaining build
requirements:

```bash
sudo apt install linux-headers-$(uname -r) device-tree-compiler raspi-utils
```

Clone and build the official Pi 5 branch:

```bash
cd ~/dev
git clone --branch pi5 --single-branch \
  https://github.com/jgarff/rpi_ws281x.git
cd ~/dev/rpi_ws281x/rp1_ws281x_pwm
make
./dts.sh
```

Install Orion's persistent configuration from the Orion checkout:

```bash
cd ~/dev/orion
sudo hardware/lighting/install-persistent.sh /home/mofe/dev/rpi_ws281x
sudo reboot
```

The installer performs six persistent operations:

1. Installs the kernel-matched module under `/lib/modules/$(uname -r)/extra/`
   and refreshes module dependencies.
2. Installs `rp1_ws281x_pwm.dtbo` into the Pi boot overlay directory and adds
   `dtoverlay=rp1_ws281x_pwm` to `config.txt`.
3. Configures `rp1_ws281x_pwm` to use PWM channel 0 through
   `/etc/modprobe.d/orion-neopixel.conf`.
4. Loads the module at boot through
   `/etc/modules-load.d/orion-neopixel.conf`.
5. Installs a udev rule granting the Raspberry Pi `gpio` group read/write
   access to `/dev/ws281x_pwm`, allowing source-run development without a root
   daemon.
6. Enables `orion-neopixel-pin.service`, which assigns BCM12 to RP1 function
   `a0` and verifies that `/dev/ws281x_pwm` exists.

Before changing `config.txt`, the installer preserves its original contents as
`config.txt.orion-backup`. The installer is idempotent and can be rerun.

After the reboot, verify the complete boot contract:

```bash
cd ~/dev/orion
hardware/lighting/verify-persistent.sh
```

Run `id -nG` once to confirm the development user belongs to the `gpio` group.
The verifier requires readable and writable `/dev/ws281x_pwm`, the loaded
module with `pwm_channel=0`, BCM12 configured as `PWM0_CHAN0`, and an enabled
and active pin service. It prints `PASS` only when the complete contract holds.

The module is compiled for one kernel ABI. After booting a newly installed
kernel, rebuild `rp1_ws281x_pwm.ko` against that running kernel's headers and
rerun the installer. Lighting remains unavailable between that kernel change
and the rebuild.

## Orion output checks

Build the Rust runtime on the Pi, then verify the four logical channels with
pixel 0 before lighting the full matrix:

```bash
cargo build --release --manifest-path runtime/Cargo.toml

runtime/target/release/oriond --lights-off
runtime/target/release/oriond --light-pixel 0 8 0 0 0
runtime/target/release/oriond --light-pixel 0 0 8 0 0
runtime/target/release/oriond --light-pixel 0 0 0 8 0
runtime/target/release/oriond --light-pixel 0 0 0 0 8
runtime/target/release/oriond --light 0 0 0 8
runtime/target/release/oriond --lights-off
```

`Pi5NeoPixelDevice` is behind the portable `LightingDevice` interface. It
encodes logical RGBW as the shield's GRBW wire order and writes the exact
40-pixel RP1 PWM frame to `/dev/ws281x_pwm`. `--lighting-device PATH` can
override that path for diagnostics. These direct commands do not require the
servo daemon and provide the physical-light commissioning surface. When the
source-run daemon is active, it owns the device exclusively and scenes become
the normal semantic lighting interface.
