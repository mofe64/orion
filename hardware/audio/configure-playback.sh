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

# The V2 HAT's JST speaker amplifier is fed by the codec's differential right
# line output. -6 dB is the commissioned Orion speaker level; the analogue
# route remains at unity gain.
amixer -q -c "${card_name}" -- sset 'PCM' -6dB
amixer -q -c "${card_name}" -- sset 'Right DAC Mux' 'DAC_R1'
amixer -q -c "${card_name}" -- sset 'Right Line Mixer DACR1' on
amixer -q -c "${card_name}" -- sset 'Line DAC' 0dB
amixer -q -c "${card_name}" -- sset 'Line' 0dB unmute

echo "Configured Orion ReSpeaker V2 JST playback on ALSA card ${card_name}."
amixer -c "${card_name}" sget 'PCM'
