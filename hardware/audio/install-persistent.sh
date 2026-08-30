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

for required_command in cp grep install modinfo sed uname; do
    if ! command -v "${required_command}" >/dev/null 2>&1; then
        echo "Required command is not installed: ${required_command}" >&2
        exit 1
    fi
done

upstream_root=$1
overlay_name=respeaker-2mic-v2_0
overlay_source_dts=${upstream_root}/overlays/rpi/${overlay_name}-overlay.dts
overlay_source_dtbo=${upstream_root}/overlays/rpi/${overlay_name}-overlay.dtbo

if [[ ${upstream_root} != /* ]]; then
    echo "The seeed-linux-dtoverlays path must be absolute." >&2
    exit 2
fi

if [[ ! -f ${overlay_source_dts} || ! -f ${overlay_source_dtbo} ]]; then
    echo "Missing the built ReSpeaker V2 overlay under ${upstream_root}/overlays/rpi/." >&2
    echo "Build it with: make overlays/rpi/${overlay_name}-overlay.dtbo" >&2
    exit 1
fi

if [[ ${overlay_source_dts} -nt ${overlay_source_dtbo} ]]; then
    echo "The ReSpeaker overlay source is newer than the compiled overlay." >&2
    echo "Rebuild it with: make overlays/rpi/${overlay_name}-overlay.dtbo" >&2
    exit 1
fi

if ! grep -q 'i2s_clk_consumer' "${overlay_source_dts}"; then
    echo "The Seeed V2 overlay does not select the Raspberry Pi 5 I2S clock-consumer block." >&2
    echo "Update the seeed-linux-dtoverlays checkout and rebuild the overlay." >&2
    exit 1
fi

if ! grep -q 'ti,tlv320aic3104' "${overlay_source_dts}" ||
    ! grep -Eq 'reg[[:space:]]*=[[:space:]]*<0x18>' "${overlay_source_dts}"; then
    echo "The selected overlay is not the TLV320AIC3104 ReSpeaker V2 overlay at I2C address 0x18." >&2
    exit 1
fi

if ! modinfo snd_soc_tlv320aic3x >/dev/null 2>&1; then
    echo "The running kernel does not provide snd_soc_tlv320aic3x." >&2
    echo "Install the matching Raspberry Pi kernel modules before continuing." >&2
    exit 1
fi

if ! modinfo snd_soc_tlv320aic3x_i2c >/dev/null 2>&1; then
    echo "The running kernel does not provide snd_soc_tlv320aic3x_i2c." >&2
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

conflicting_overlay_pattern='^[[:space:]]*dtoverlay=(seeed-2mic-voicecard|wm8960-soundcard)(,|[[:space:]]|$)'
if grep -Eq "${conflicting_overlay_pattern}" "${boot_config}"; then
    echo "A conflicting ReSpeaker/audio overlay is already configured in ${boot_config}:" >&2
    grep -En "${conflicting_overlay_pattern}" "${boot_config}" >&2
    echo "Remove or reconcile that entry before installing Orion's V2 overlay." >&2
    exit 1
fi

install -m 0644 "${overlay_source_dtbo}" \
    "${boot_overlay_directory}/${overlay_name}.dtbo"

if [[ ! -e ${boot_config}.orion-audio-backup ]]; then
    cp -a "${boot_config}" "${boot_config}.orion-audio-backup"
fi

# Orion previously installed the V1 entry before the physical codec was
# identified at 0x18. Remove only that exact Orion-owned setting during the
# V2 migration; leave every unrelated boot setting untouched.
sed -i \
    -e '/^[[:space:]]*# Orion ReSpeaker 2-Mics Pi HAT V1 \/ WM8960 audio[[:space:]]*$/d' \
    -e '/^[[:space:]]*dtoverlay=respeaker-2mic-v1_0[[:space:]]*$/d' \
    "${boot_config}"

if ! grep -Eq "^[[:space:]]*dtoverlay=${overlay_name}([,[:space:]]|$)" "${boot_config}"; then
    {
        printf '\n# Orion ReSpeaker 2-Mics Pi HAT V2 / TLV320AIC3104 audio\n'
        printf 'dtoverlay=%s\n' "${overlay_name}"
    } >> "${boot_config}"
fi

echo "Installed persistent Orion ReSpeaker V2 TLV320AIC3104 overlay for kernel $(uname -r)."
echo "Boot configuration: ${boot_config}"
if [[ -e ${boot_config}.orion-audio-backup ]]; then
    echo "Boot configuration backup: ${boot_config}.orion-audio-backup"
fi
echo "Reboot, then run hardware/audio/verify-persistent.sh."
