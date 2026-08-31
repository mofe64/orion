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
asr_parent="${model_root}/asr"
asr_name=sherpa-onnx-moonshine-tiny-en-int8
asr_directory="${asr_parent}/${asr_name}"
asr_archive_url="https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/${asr_name}.tar.bz2"
vad_directory="${model_root}/vad"
vad_model="${vad_directory}/silero_vad.onnx"
vad_model_url="https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx"
temporary_downloads=()

cleanup_downloads() {
    if [[ ${#temporary_downloads[@]} -gt 0 ]]; then
        rm -f -- "${temporary_downloads[@]}"
    fi
}
trap cleanup_downloads EXIT

if [[ ! -x ${voice_python} ]] || [[ ! -x ${sherpa_cli} ]]; then
    echo "Install voice dependencies first:" >&2
    echo "  /home/mofe/.local/bin/uv sync --project voice --python 3.11" >&2
    exit 1
fi

mkdir -p "${model_root}"
"${voice_python}" -m piper.download_voices \
    --download-dir "${model_root}" \
    en_US-ryan-medium

wake_model_complete=true
for required_file in \
    tokens.txt \
    bpe.model \
    encoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx \
    decoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx \
    joiner-epoch-12-avg-2-chunk-16-left-64.int8.onnx; do
    if [[ ! -f ${wake_directory}/${required_file} ]]; then
        wake_model_complete=false
    fi
done

if [[ ${wake_model_complete} != true ]]; then
    mkdir -p "${wake_parent}"
    archive_path=$(mktemp --tmpdir orion-wake-model.XXXXXX.tar.bz2)
    temporary_downloads+=("${archive_path}")
    curl --fail --location "${wake_archive_url}" --output "${archive_path}"
    tar -xjf "${archive_path}" -C "${wake_parent}"
fi

"${repository_root}/voice/configure-wake-word.sh" "HELLO WORLD"

asr_model_complete=true
for required_file in \
    preprocess.onnx \
    encode.int8.onnx \
    uncached_decode.int8.onnx \
    cached_decode.int8.onnx \
    tokens.txt; do
    if [[ ! -f ${asr_directory}/${required_file} ]]; then
        asr_model_complete=false
    fi
done

if [[ ${asr_model_complete} != true ]]; then
    mkdir -p "${asr_parent}"
    archive_path=$(mktemp --tmpdir orion-asr-model.XXXXXX.tar.bz2)
    temporary_downloads+=("${archive_path}")
    curl --fail --location "${asr_archive_url}" --output "${archive_path}"
    tar -xjf "${archive_path}" -C "${asr_parent}"
fi

if [[ ! -f ${vad_model} ]]; then
    mkdir -p "${vad_directory}"
    vad_temporary=$(mktemp --tmpdir="${vad_directory}" .silero-vad.XXXXXX.onnx)
    temporary_downloads+=("${vad_temporary}")
    curl --fail --location "${vad_model_url}" --output "${vad_temporary}"
    mv -- "${vad_temporary}" "${vad_model}"
fi

"${repository_root}/voice/cleanup-voices.sh"
echo "Installed Ryan Medium, HELLO WORLD wake detection, Silero VAD, and Moonshine Tiny English ASR."
