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

for required_command in modinfo depmod install grep systemctl udevadm getent dkms dtc perl; do
    if ! command -v "${required_command}" >/dev/null 2>&1; then
        echo "Required command is not installed: ${required_command}" >&2
        exit 1
    fi
done

if [[ ! -x /usr/bin/pinctrl ]]; then
    echo "Required Raspberry Pi utility is missing: /usr/bin/pinctrl" >&2
    exit 1
fi

if ! getent group gpio >/dev/null; then
    echo "Required Raspberry Pi group does not exist: gpio" >&2
    exit 1
fi

upstream_root=$1
driver_directory=${upstream_root}/rp1_ws281x_pwm
kernel_release=$(uname -r)
module_source=${driver_directory}/rp1_ws281x_pwm.c
overlay_source=${driver_directory}/rp1_ws281x_pwm.dts
script_directory=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)

if [[ ${upstream_root} != /* ]]; then
    echo "The rpi_ws281x path must be absolute." >&2
    exit 2
fi

if [[ ! -f ${module_source} || ! -f ${overlay_source} ]]; then
    echo "Missing Pi 5 driver source or overlay source in ${driver_directory}." >&2
    echo "Clone the rpi_ws281x pi5 branch at ${upstream_root} first." >&2
    exit 1
fi

# Future kernels need their headers installed alongside them, or DKMS cannot
# rebuild the driver when they arrive.
if [[ -r /etc/os-release ]] && grep -q '^ID=ubuntu' /etc/os-release \
    && ! dpkg-query -W -f='${Status}' linux-headers-raspi 2>/dev/null | grep -q 'install ok installed'; then
    echo "Install linux-headers-raspi so each kernel update brings matching headers:" >&2
    echo "  sudo apt install linux-headers-raspi" >&2
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

# Register the driver source with DKMS. The kernel package's postinstall hook
# then builds and installs it for each new kernel before that kernel boots.
dkms_name=rp1_ws281x_pwm
dkms_version=orion1
dkms_source=/usr/src/${dkms_name}-${dkms_version}
# Earlier installs copied a single-kernel build here. Remove it before DKMS
# installs its own copy, which may use the same directory.
rm -f "/lib/modules/${kernel_release}/extra/rp1_ws281x_pwm.ko"
if dkms status "${dkms_name}/${dkms_version}" | grep -q .; then
    dkms remove "${dkms_name}/${dkms_version}" --all
fi
rm -rf "${dkms_source}"
install -d "${dkms_source}"
install -m 0644 "${driver_directory}"/*.c "${driver_directory}"/*.h \
    "${driver_directory}/Makefile" "${dkms_source}/"
install -m 0644 "${script_directory}/dkms.conf" "${dkms_source}/dkms.conf"
# Upstream declares remove() as returning void, which kernels before 6.11 only
# accept through .remove_new. Select the field by kernel version so the same
# source builds on Ubuntu's 6.8 kernel and on later kernels.
driver_copy=${dkms_source}/rp1_ws281x_pwm.c
if grep -q '^void rp1_ws281x_pwm_remove' "${driver_copy}" \
    && ! grep -q 'remove_new' "${driver_copy}"; then
    perl -0pi -e 's/#include <linux\/kernel.h>\n/#include <linux\/kernel.h>\n#include <linux\/version.h>\n/; s/^(\t)\.remove = rp1_ws281x_pwm_remove,$/#if LINUX_VERSION_CODE < KERNEL_VERSION(6, 11, 0)\n$1.remove_new = rp1_ws281x_pwm_remove,\n#else\n$1.remove = rp1_ws281x_pwm_remove,\n#endif/m' "${driver_copy}"
fi
dkms add "${dkms_name}/${dkms_version}"
dkms build "${dkms_name}/${dkms_version}" -k "${kernel_release}"
dkms install "${dkms_name}/${dkms_version}" -k "${kernel_release}" --force
depmod -a "${kernel_release}"

# Kernel releases name the RP1 node differently (/axi/pcie@120000/rp1 on
# Ubuntu's 6.8, /axi/pcie@1000120000/rp1 on later Raspberry Pi kernels), and
# firmware silently skips an overlay whose target path does not exist. Target
# the node found in the running device tree. When the base tree labels that
# node rp1, target the label instead so the overlay follows future renames.
rp1_node=$(cd /proc/device-tree && ls -d axi/pcie@*/rp1 2>/dev/null | head -n 1)
if [[ -z ${rp1_node} ]]; then
    echo "Could not find the RP1 node in /proc/device-tree. Run this on a Pi 5." >&2
    exit 1
fi
rp1_label_path=$(tr -d '\0' < /proc/device-tree/__symbols__/rp1 2>/dev/null || true)
if [[ ${rp1_label_path} == "/${rp1_node}" ]]; then
    overlay_target='target = <&rp1>;'
else
    overlay_target="target-path = \"/${rp1_node}\";"
fi
overlay_build=$(mktemp -d)
trap 'rm -rf "${overlay_build}"' EXIT
OVERLAY_TARGET=${overlay_target} perl -pe \
    's/^(\s*)target(-path)?\s*=.*;/$1$ENV{OVERLAY_TARGET}/' \
    "${overlay_source}" > "${overlay_build}/rp1_ws281x_pwm.dts"
dtc -q -@ -O dtb -o "${overlay_build}/rp1_ws281x_pwm.dtbo" \
    "${overlay_build}/rp1_ws281x_pwm.dts"
echo "Overlay ${overlay_target}"
install -m 0644 "${overlay_build}/rp1_ws281x_pwm.dtbo" \
    "${boot_overlay_directory}/rp1_ws281x_pwm.dtbo"
install -m 0644 "${script_directory}/orion-neopixel-modprobe.conf" \
    /etc/modprobe.d/orion-neopixel.conf
install -m 0644 "${script_directory}/orion-neopixel.modules" \
    /etc/modules-load.d/orion-neopixel.conf
install -m 0644 "${script_directory}/orion-neopixel-pin.service" \
    /etc/systemd/system/orion-neopixel-pin.service
install -m 0644 "${script_directory}/70-orion-neopixel.rules" \
    /etc/udev/rules.d/70-orion-neopixel.rules

if ! grep -Eq '^[[:space:]]*dtoverlay=rp1_ws281x_pwm([[:space:]]|$)' "${boot_config}"; then
    if [[ ! -e ${boot_config}.orion-backup ]]; then
        cp -a "${boot_config}" "${boot_config}.orion-backup"
    fi
    {
        printf '\n# Orion GPIO12 RGBW matrix/ring RP1 PWM device\n'
        printf 'dtoverlay=rp1_ws281x_pwm\n'
    } >> "${boot_config}"
fi

systemctl daemon-reload
systemctl enable orion-neopixel-pin.service
udevadm control --reload-rules
if [[ -e /sys/class/misc/ws281x_pwm ]]; then
    udevadm trigger --action=add /sys/class/misc/ws281x_pwm
fi

echo "Installed persistent Orion NeoPixel support for kernel ${kernel_release}."
echo "DKMS rebuilds the driver for each kernel update: $(dkms status "${dkms_name}/${dkms_version}" | tr '\n' ' ')"
echo "Boot configuration: ${boot_config}"
if [[ -e ${boot_config}.orion-backup ]]; then
    echo "Boot configuration backup: ${boot_config}.orion-backup"
fi
echo "Reboot, then run hardware/lighting/verify-persistent.sh."
