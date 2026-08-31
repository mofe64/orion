#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 'WAKE PHRASE'" >&2
    exit 2
fi

repository_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
sherpa_cli="${repository_root}/voice/.venv/bin/sherpa-onnx-cli"
wake_name=sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01
wake_directory="${repository_root}/voice/models/wake/${wake_name}"
wake_phrase=${1^^}

if [[ ! ${wake_phrase} =~ ^[A-Z]+([[:space:]][A-Z]+)*$ ]]; then
    echo "Wake phrase must contain English letters separated by single spaces." >&2
    exit 2
fi

for required_path in \
    "${sherpa_cli}" \
    "${wake_directory}/tokens.txt" \
    "${wake_directory}/bpe.model"; do
    if [[ ! -e ${required_path} ]]; then
        echo "Missing wake-word dependency: ${required_path}" >&2
        echo "Run voice/install-models.sh first." >&2
        exit 1
    fi
done

raw_temporary=$(mktemp --tmpdir="${wake_directory}" .orion-keywords-raw.XXXXXX)
tokens_temporary=$(mktemp --tmpdir="${wake_directory}" .orion-keywords.XXXXXX)
trap 'rm -f -- "${raw_temporary}" "${tokens_temporary}"' EXIT

printf '%s\n' "${wake_phrase}" > "${raw_temporary}"
"${sherpa_cli}" text2token \
    --tokens "${wake_directory}/tokens.txt" \
    --tokens-type bpe \
    --bpe-model "${wake_directory}/bpe.model" \
    "${raw_temporary}" \
    "${tokens_temporary}"

mv -- "${raw_temporary}" "${wake_directory}/orion_keywords_raw.txt"
mv -- "${tokens_temporary}" "${wake_directory}/orion_keywords.txt"
chmod 0644 \
    "${wake_directory}/orion_keywords_raw.txt" \
    "${wake_directory}/orion_keywords.txt"

echo "Configured Orion wake phrase: ${wake_phrase}"
cat "${wake_directory}/orion_keywords.txt"
