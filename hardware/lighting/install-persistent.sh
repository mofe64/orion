#!/usr/bin/env bash

set -euo pipefail

if [[ ${EUID} -ne 0 ]]; then
    echo "Run this installer with sudo." >&2
    exit 1
fi

if [[ $# -ne 1 ]]; then
    echo "Usage: sudo $0 /absolute/path/to/rpi_ws281x" >&2
    exit 2
fi

for required_command in modinfo depmod install grep systemctl; do
    if ! command -v "${required_command}" >/dev/null 2>&1; then
        echo "Required command is not installed: ${required_command}" >&2
        exit 1
    fi
done

if [[ ! -x /usr/bin/pinctrl ]]; then
    echo "Required Raspberry Pi utility is missing: /usr/bin/pinctrl" >&2
    exit 1
fi

upstream_root=$1
driver_directory=${upstream_root}/rp1_ws281x_pwm
kernel_release=$(uname -r)
module_source=${driver_directory}/rp1_ws281x_pwm.ko
overlay_source=${driver_directory}/rp1_ws281x_pwm.dtbo
script_directory=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)

if [[ ${upstream_root} != /* ]]; then
    echo "The rpi_ws281x path must be absolute." >&2
    exit 2
fi

if [[ ! -f ${module_source} || ! -f ${overlay_source} ]]; then
    echo "Missing built Pi 5 module or overlay in ${driver_directory}." >&2
    echo "Build the rpi_ws281x pi5 branch with 'make' and './dts.sh' first." >&2
    exit 1
fi

module_vermagic=$(modinfo -F vermagic "${module_source}")
if [[ ${module_vermagic%% *} != "${kernel_release}" ]]; then
    echo "Module vermagic '${module_vermagic}' does not match running kernel '${kernel_release}'." >&2
    exit 1
fi

if [[ -d /boot/firmware/overlays && -f /boot/firmware/config.txt ]]; then
    boot_overlay_directory=/boot/firmware/overlays
    boot_config=/boot/firmware/config.txt
elif [[ -d /boot/overlays && -f /boot/config.txt ]]; then
    boot_overlay_directory=/boot/overlays
    boot_config=/boot/config.txt
else
    echo "Could not find the Raspberry Pi boot overlay directory and config.txt." >&2
    exit 1
fi

install -D -m 0644 "${module_source}" \
    "/lib/modules/${kernel_release}/extra/rp1_ws281x_pwm.ko"
depmod -a "${kernel_release}"

install -m 0644 "${overlay_source}" \
    "${boot_overlay_directory}/rp1_ws281x_pwm.dtbo"
install -m 0644 "${script_directory}/orion-neopixel-modprobe.conf" \
    /etc/modprobe.d/orion-neopixel.conf
install -m 0644 "${script_directory}/orion-neopixel.modules" \
    /etc/modules-load.d/orion-neopixel.conf
install -m 0644 "${script_directory}/orion-neopixel-pin.service" \
    /etc/systemd/system/orion-neopixel-pin.service

if ! grep -Eq '^[[:space:]]*dtoverlay=rp1_ws281x_pwm([[:space:]]|$)' "${boot_config}"; then
    if [[ ! -e ${boot_config}.orion-backup ]]; then
        cp -a "${boot_config}" "${boot_config}.orion-backup"
    fi
    {
        printf '\n# Orion 40-pixel RGBW shield RP1 PWM device\n'
        printf 'dtoverlay=rp1_ws281x_pwm\n'
    } >> "${boot_config}"
fi

systemctl daemon-reload
systemctl enable orion-neopixel-pin.service

echo "Installed persistent Orion NeoPixel support for kernel ${kernel_release}."
echo "Boot configuration: ${boot_config}"
if [[ -e ${boot_config}.orion-backup ]]; then
    echo "Boot configuration backup: ${boot_config}.orion-backup"
fi
echo "Reboot, then run hardware/lighting/verify-persistent.sh."
