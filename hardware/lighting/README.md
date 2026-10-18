# Orion lighting hardware

V1 uses a 40-pixel RGBW matrix. V2 uses a 24-pixel RGBW ring. RGBW gives each pixel separate red, green, blue and white channels.

Both profiles use GPIO12 and the Raspberry Pi 5 RP1 PWM driver. Pulse-width modulation, or PWM, controls signal timing. The device path is `/dev/ws281x_pwm`.

## Wiring

The V1 shield uses these header connections:

```text
Pi pin 4:  5 V     → shield 5 V
Pi pin 30: ground → shield ground
Pi pin 32: GPIO12 → shield DIN
```

The V1 matrix uses this row-major order:

```text
 0  1  2  3  4  5  6  7
 8  9 10 11 12 13 14 15
16 17 18 19 20 21 22 23
24 25 26 27 28 29 30 31
32 33 34 35 36 37 38 39
```

The driver converts logical RGBW values to GRBW wire order. V2 ring indices run from 0 to 23. The selected hardware profile determines pixel count.

CAUTION: Incorrect wiring can damage the Pi or LEDs. Disconnect power before changing connections.

1. Check the board's supply, ground and data connections before applying power.

## Persistent Raspberry Pi 5 setup

The [release installer](../../docs/quickstart.md) installs the lighting driver. Manual source setup requires Raspberry Pi OS Bookworm or Trixie.

1. Install the driver build packages and Pi 5 headers.

   ```bash
   sudo apt-get install -y dkms linux-image-rpi-2712 linux-headers-rpi-2712 device-tree-compiler raspi-utils git
   ```

2. Add the login user to the GPIO group.

   ```bash
   sudo usermod -aG gpio "$USER"
   ```

3. Clone the upstream Pi 5 driver sources.

   ```bash
   git clone --branch pi5 --single-branch https://github.com/jgarff/rpi_ws281x.git "$HOME/rpi_ws281x"
   ```

4. Install persistent lighting configuration from the Orion repository root.

   ```bash
   sudo hardware/lighting/install-persistent.sh "$HOME/rpi_ws281x"
   ```

5. Reboot the Pi.

   ```bash
   sudo reboot
   ```

6. Check the installed module, pin function and device permissions.

   ```bash
   hardware/lighting/verify-persistent.sh
   ```

The installer registers `rp1_ws281x_pwm/orion1` with DKMS. DKMS rebuilds kernel modules when matching kernel headers are installed. The installer builds for available Pi 5 header trees.

It installs the boot overlay, module options and GPIO permissions. `orion-neopixel-pin.service` assigns GPIO12 to PWM channel 0. The original boot file remains in `config.txt.orion-backup`.

## Kernel updates

1. List driver builds after a kernel update.

   ```bash
   dkms status rp1_ws281x_pwm
   ```

2. Build a missing kernel module after installing that kernel's headers.

   ```bash
   sudo dkms autoinstall -k KERNEL
   ```

## Light controls

`oriond` owns the device while the daemon runs. Named scene effects provide normal animated lighting.

The Lamp panel in Studio follows the setting on Orion. Voice changes and changes from other Studio windows appear automatically.
The controls show the chosen power, brightness, mood, colours and effect. A fade shows its remaining time.

Temporary speech, idle and scene lighting does not replace the chosen setting. Studio shows a short message during these changes.
Brightness changes keep the chosen colours and effect. The power switch keeps the chosen brightness and palette.

CAUTION: High output can overload the LED supply. Start with low channel values when checking wiring.

1. Set one V2 pixel to low red output.

   ```bash
   oriond --hardware v2 --light-pixel 0 8 0 0 0
   ```

2. Turn the light off.

   ```bash
   oriond --hardware v2 --lights-off
   ```

Each channel accepts 0–255. The runtime does not impose a separate brightness ceiling. Persistent flicker or incorrect colors can indicate a wiring or signal-level problem.
