#!/usr/bin/env bash

set -euo pipefail

if [[ ${EUID} -ne 0 ]]; then
    echo "Run this installer with sudo." >&2
    exit 1
fi

if [[ $# -ne 1 ]]; then
    echo "Usage: sudo $0 /absolute/path/to/seeed-linux-dtoverlays" >&2
    exit 2
fi

for required_command in cp grep install modinfo uname; do
    if ! command -v "${required_command}" >/dev/null 2>&1; then
        echo "Required command is not installed: ${required_command}" >&2
        exit 1
    fi
done

upstream_root=$1
overlay_source_dts=${upstream_root}/overlays/rpi/respeaker-2mic-v1_0-overlay.dts
overlay_source_dtbo=${upstream_root}/overlays/rpi/respeaker-2mic-v1_0-overlay.dtbo

if [[ ${upstream_root} != /* ]]; then
    echo "The seeed-linux-dtoverlays path must be absolute." >&2
    exit 2
fi

if [[ ! -f ${overlay_source_dts} || ! -f ${overlay_source_dtbo} ]]; then
    echo "Missing the built ReSpeaker V1 overlay under ${upstream_root}/overlays/rpi/." >&2
    echo "Build it with: make overlays/rpi/respeaker-2mic-v1_0-overlay.dtbo" >&2
    exit 1
fi

if [[ ${overlay_source_dts} -nt ${overlay_source_dtbo} ]]; then
    echo "The ReSpeaker overlay source is newer than the compiled overlay." >&2
    echo "Rebuild it with: make overlays/rpi/respeaker-2mic-v1_0-overlay.dtbo" >&2
    exit 1
fi

if ! grep -q 'brcm,bcm2712' "${overlay_source_dts}"; then
    echo "The Seeed V1 overlay does not declare Raspberry Pi 5 (bcm2712) support." >&2
    echo "Update the seeed-linux-dtoverlays checkout and rebuild the overlay." >&2
    exit 1
fi

if ! grep -q 'wlf,wm8960' "${overlay_source_dts}"; then
    echo "The selected overlay is not the WM8960 ReSpeaker V1 overlay." >&2
    exit 1
fi

if ! modinfo snd_soc_wm8960 >/dev/null 2>&1; then
    echo "The running kernel does not provide snd_soc_wm8960." >&2
    echo "Install the matching Raspberry Pi kernel modules before continuing." >&2
    exit 1
fi

if ! modinfo snd_soc_simple_card >/dev/null 2>&1; then
    echo "The running kernel does not provide snd_soc_simple_card." >&2
    echo "Install the matching Raspberry Pi kernel modules before continuing." >&2
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

conflicting_overlay_pattern='^[[:space:]]*dtoverlay=(respeaker-2mic-v2_0|seeed-2mic-voicecard|wm8960-soundcard)(,|[[:space:]]|$)'
if grep -Eq "${conflicting_overlay_pattern}" "${boot_config}"; then
    echo "A conflicting WM8960/ReSpeaker overlay is already configured in ${boot_config}:" >&2
    grep -En "${conflicting_overlay_pattern}" "${boot_config}" >&2
    echo "Remove or reconcile that entry before installing Orion's V1 overlay." >&2
    exit 1
fi

install -m 0644 "${overlay_source_dtbo}" \
    "${boot_overlay_directory}/respeaker-2mic-v1_0.dtbo"

if ! grep -Eq '^[[:space:]]*dtoverlay=respeaker-2mic-v1_0([,[:space:]]|$)' "${boot_config}"; then
    if [[ ! -e ${boot_config}.orion-audio-backup ]]; then
        cp -a "${boot_config}" "${boot_config}.orion-audio-backup"
    fi
    {
        printf '\n# Orion ReSpeaker 2-Mics Pi HAT V1 / WM8960 audio\n'
        printf 'dtoverlay=respeaker-2mic-v1_0\n'
    } >> "${boot_config}"
fi

echo "Installed persistent Orion ReSpeaker WM8960 overlay for kernel $(uname -r)."
echo "Boot configuration: ${boot_config}"
if [[ -e ${boot_config}.orion-audio-backup ]]; then
    echo "Boot configuration backup: ${boot_config}.orion-audio-backup"
fi
echo "Reboot, then run hardware/audio/verify-persistent.sh."
