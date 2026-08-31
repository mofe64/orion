#!/usr/bin/env bash

set -euo pipefail

card_name=${1:-seeed2micvoicec}

if ! command -v amixer >/dev/null 2>&1; then
    echo "amixer is required; install the alsa-utils package." >&2
    exit 1
fi

if ! amixer -q -c "${card_name}" info >/dev/null 2>&1; then
    echo "ALSA card '${card_name}' is not available." >&2
    exit 1
fi

# The V2 HAT routes its left and right analogue microphones through LINE1L and
# LINE1R. Use a fixed gain so wake-word behavior does not depend on inherited
# mixer state. Avoid the codec AGC: its Linux driver warns that it can raise the
# PGA to maximum and leave it there while the ADC remains active.
amixer -q -c "${card_name}" -- sset 'AGC' off
amixer -q -c "${card_name}" -- sset 'Left Line1L Mux' 'single-ended'
amixer -q -c "${card_name}" -- sset 'Right Line1R Mux' 'single-ended'
amixer -q -c "${card_name}" -- sset 'Left PGA Mixer Line1L' on
amixer -q -c "${card_name}" -- sset 'Right PGA Mixer Line1R' on
amixer -q -c "${card_name}" -- sset 'PGA' 40dB unmute

echo "Configured Orion ReSpeaker V2 microphone capture on ALSA card ${card_name}."
amixer -c "${card_name}" sget 'PGA'
