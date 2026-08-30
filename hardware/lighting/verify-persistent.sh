#!/usr/bin/env bash

set -euo pipefail

device=/dev/ws281x_pwm

if [[ ! -c ${device} ]]; then
    echo "FAIL: ${device} is not a character device." >&2
    exit 1
fi

if [[ ! -r ${device} || ! -w ${device} ]]; then
    echo "FAIL: the current user cannot read and write ${device}." >&2
    echo "Ensure the user belongs to the gpio group, then start a new login session." >&2
    exit 1
fi

if ! grep -q '^rp1_ws281x_pwm ' /proc/modules; then
    echo "FAIL: rp1_ws281x_pwm is not loaded." >&2
    exit 1
fi

channel_parameter=/sys/module/rp1_ws281x_pwm/parameters/pwm_channel
if [[ ! -r ${channel_parameter} ]] || [[ $(<"${channel_parameter}") != 0 ]]; then
    echo "FAIL: rp1_ws281x_pwm is not configured for PWM channel 0." >&2
    exit 1
fi

pin_state=$(pinctrl get 12)
if [[ ${pin_state} != *"a0"* || ${pin_state} != *"PWM0_CHAN0"* ]]; then
    echo "FAIL: BCM12 is not configured as RP1 PWM0 channel 0: ${pin_state}" >&2
    exit 1
fi

if ! systemctl is-enabled --quiet orion-neopixel-pin.service; then
    echo "FAIL: orion-neopixel-pin.service is not enabled." >&2
    exit 1
fi

if ! systemctl is-active --quiet orion-neopixel-pin.service; then
    echo "FAIL: orion-neopixel-pin.service is not active." >&2
    exit 1
fi

echo "PASS: persistent Orion NeoPixel boot configuration is active."
echo "rp1_ws281x_pwm pwm_channel=$(<"${channel_parameter}")"
echo "${pin_state}"
ls -l "${device}"
