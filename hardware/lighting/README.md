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
initial GRBW/800 kHz configuration. Pixel order and the physical matrix index
direction remain commissioning observations rather than assumptions.

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

## One-time Raspberry Pi 5 setup

Raspberry Pi 5 support in `rpi_ws281x` uses the RP1 PWM kernel module from its
`pi5` branch. On the Pi, install its build requirements, clone that branch,
and build the driver and overlay:

```bash
sudo apt install linux-headers device-tree-compiler raspi-utils
git clone --branch pi5 https://github.com/jgarff/rpi_ws281x.git
cd rpi_ws281x/rp1_ws281x_pwm
make
./dts.sh
```

For Orion's BCM12 connection, load PWM channel 0, apply the overlay, and route
GPIO12 to RP1 PWM function `a0`:

```bash
sudo insmod ./rp1_ws281x_pwm.ko pwm_channel=0
sudo dtoverlay -d . rp1_ws281x_pwm
sudo pinctrl set 12 a0 pn
ls -l /dev/ws281x_pwm
```

These load commands must be made persistent by the robot's service setup after
commissioning; they do not survive a reboot as written.

## Orion output checks

Build the Rust runtime on the Pi, then verify the four logical channels with
pixel 0 before lighting the full matrix:

```bash
cargo build --release --manifest-path runtime/Cargo.toml

sudo runtime/target/release/oriond --lights-off
sudo runtime/target/release/oriond --light-pixel 0 8 0 0 0
sudo runtime/target/release/oriond --light-pixel 0 0 8 0 0
sudo runtime/target/release/oriond --light-pixel 0 0 0 8 0
sudo runtime/target/release/oriond --light-pixel 0 0 0 0 8
sudo runtime/target/release/oriond --light 0 0 0 8
sudo runtime/target/release/oriond --lights-off
```

`Pi5NeoPixelDevice` is behind the portable `LightingDevice` interface. It
encodes logical RGBW as the shield's GRBW wire order and writes the exact
40-pixel RP1 PWM frame to `/dev/ws281x_pwm`. `--lighting-device PATH` can
override that path for diagnostics. These direct commands do not require the
servo daemon and currently provide the physical-light commissioning surface;
daemon-owned scene playback is the next integration boundary.
