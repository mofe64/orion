#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
voice_python="${repository_root}/voice/.venv/bin/python"
sherpa_cli="${repository_root}/voice/.venv/bin/sherpa-onnx-cli"
model_root="${repository_root}/voice/models"
wake_parent="${model_root}/wake"
wake_name=sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01
wake_directory="${wake_parent}/${wake_name}"
wake_archive_url="https://github.com/k2-fsa/sherpa-onnx/releases/download/kws-models/${wake_name}.tar.bz2"

if [[ ! -x ${voice_python} ]] || [[ ! -x ${sherpa_cli} ]]; then
    echo "Install voice dependencies first:" >&2
    echo "  /home/mofe/.local/bin/uv sync --project voice --python 3.11" >&2
    exit 1
fi

mkdir -p "${model_root}"
"${voice_python}" -m piper.download_voices \
    --download-dir "${model_root}" \
    en_US-ryan-medium

if [[ ! -d ${wake_directory} ]]; then
    mkdir -p "${wake_parent}"
    archive_path=$(mktemp --tmpdir orion-wake-model.XXXXXX.tar.bz2)
    trap 'rm -f -- "${archive_path}"' EXIT
    curl --fail --location "${wake_archive_url}" --output "${archive_path}"
    tar -xjf "${archive_path}" -C "${wake_parent}"
fi

printf 'HEY ORION\n' > "${wake_directory}/orion_keywords_raw.txt"
"${sherpa_cli}" text2token \
    --tokens "${wake_directory}/tokens.txt" \
    --tokens-type bpe \
    --bpe-model "${wake_directory}/bpe.model" \
    "${wake_directory}/orion_keywords_raw.txt" \
    "${wake_directory}/orion_keywords.txt"

"${repository_root}/voice/cleanup-voices.sh"
echo "Installed Ryan Medium and the local HEY ORION wake-word model."
