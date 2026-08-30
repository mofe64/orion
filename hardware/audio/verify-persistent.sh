#!/usr/bin/env bash

set -euo pipefail

card_name=seeed2micvoicec
overlay_name=respeaker-2mic-v2_0

for required_command in aplay arecord basename cat find grep readlink; do
    if ! command -v "${required_command}" >/dev/null 2>&1; then
        echo "FAIL: required command is not installed: ${required_command}" >&2
        exit 1
    fi
done

if [[ -d /boot/firmware/overlays && -f /boot/firmware/config.txt ]]; then
    boot_overlay_directory=/boot/firmware/overlays
    boot_config=/boot/firmware/config.txt
elif [[ -d /boot/overlays && -f /boot/config.txt ]]; then
    boot_overlay_directory=/boot/overlays
    boot_config=/boot/config.txt
else
    echo "FAIL: could not find the Raspberry Pi boot configuration." >&2
    exit 1
fi

if [[ ! -f ${boot_overlay_directory}/${overlay_name}.dtbo ]]; then
    echo "FAIL: ${overlay_name}.dtbo is not installed." >&2
    exit 1
fi

if ! grep -Eq "^[[:space:]]*dtoverlay=${overlay_name}([,[:space:]]|$)" "${boot_config}"; then
    echo "FAIL: ${overlay_name} is not enabled in ${boot_config}." >&2
    exit 1
fi

if grep -Eq '^[[:space:]]*dtoverlay=respeaker-2mic-v1_0([,[:space:]]|$)' "${boot_config}"; then
    echo "FAIL: the obsolete ReSpeaker V1 overlay is still enabled in ${boot_config}." >&2
    exit 1
fi

if [[ ! -d /sys/module/snd_soc_tlv320aic3x ]]; then
    echo "FAIL: snd_soc_tlv320aic3x is not loaded." >&2
    exit 1
fi

if [[ ! -d /sys/module/snd_soc_tlv320aic3x_i2c ]]; then
    echo "FAIL: snd_soc_tlv320aic3x_i2c is not loaded." >&2
    exit 1
fi

if [[ ! -d /sys/module/snd_soc_simple_card ]]; then
    echo "FAIL: snd_soc_simple_card is not loaded." >&2
    exit 1
fi

if ! grep -q "${card_name}" /proc/asound/cards; then
    echo "FAIL: ALSA did not register ${card_name}." >&2
    cat /proc/asound/cards >&2
    exit 1
fi

playback_devices=$(aplay -l)
if [[ ${playback_devices} != *"${card_name}"* ]]; then
    echo "FAIL: ${card_name} has no ALSA playback device." >&2
    printf '%s\n' "${playback_devices}" >&2
    exit 1
fi

capture_devices=$(arecord -l)
if [[ ${capture_devices} != *"${card_name}"* ]]; then
    echo "FAIL: ${card_name} has no ALSA capture device." >&2
    printf '%s\n' "${capture_devices}" >&2
    exit 1
fi

codec_device=$(find /sys/bus/i2c/devices -maxdepth 1 -type l -name '*-0018' -print -quit)
if [[ -z ${codec_device} || ! -L ${codec_device}/driver ]]; then
    echo "FAIL: the TLV320AIC3104 codec at I2C address 0x18 is not bound to a driver." >&2
    exit 1
fi

if [[ -c /dev/ws281x_pwm && -x /usr/bin/pinctrl ]]; then
    neopixel_pin_state=$(/usr/bin/pinctrl get 12)
    if [[ ${neopixel_pin_state} != *"PWM0_CHAN0"* ]]; then
        echo "FAIL: the audio setup displaced Orion's BCM12 NeoPixel function: ${neopixel_pin_state}" >&2
        exit 1
    fi
fi

echo "PASS: persistent Orion ReSpeaker V2 TLV320AIC3104 boot configuration is active."
echo "Codec: $(basename "${codec_device}") -> $(basename "$(readlink -f "${codec_device}/driver")")"
printf '%s\n' "${playback_devices}"
printf '%s\n' "${capture_devices}"

if [[ -n ${neopixel_pin_state:-} ]]; then
    echo "NeoPixel integration: ${neopixel_pin_state}"
fi
