#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
model_directory="${repository_root}/voice/models"
selected_voice=en_US-ryan-medium

if [[ ! -d ${model_directory} ]]; then
    echo "No Piper model directory exists at ${model_directory}."
    exit 0
fi

if [[ ! -f ${model_directory}/${selected_voice}.onnx ]] ||
   [[ ! -f ${model_directory}/${selected_voice}.onnx.json ]]; then
    echo "Refusing cleanup because Ryan Medium is incomplete. Download it with:" >&2
    echo "  voice/.venv/bin/python -m piper.download_voices --download-dir voice/models ${selected_voice}" >&2
    exit 1
fi

removed=0
shopt -s nullglob
for model_file in "${model_directory}"/*.onnx "${model_directory}"/*.onnx.json; do
    filename=$(basename "${model_file}")
    case "${filename}" in
        "${selected_voice}.onnx"|"${selected_voice}.onnx.json")
            ;;
        *)
            rm -- "${model_file}"
            echo "Removed ${filename}."
            removed=$((removed + 1))
            ;;
    esac
done

echo "Kept ${selected_voice}; removed ${removed} other Piper model files."
