from __future__ import annotations

from huggingface_hub import snapshot_download


DEFAULT_ASR_MODEL = "Qwen/Qwen3-ASR-0.6B"
DEFAULT_TTS_MODEL = "mlx-community/chatterbox-turbo-8bit"
CHATTERBOX_TOKENIZER_MODEL = "mlx-community/S3TokenizerV2"


def download_models() -> dict[str, str]:
    return {
        model: snapshot_download(repo_id=model)
        for model in (
            DEFAULT_ASR_MODEL,
            DEFAULT_TTS_MODEL,
            CHATTERBOX_TOKENIZER_MODEL,
        )
    }


def main() -> None:
    for model, path in download_models().items():
        print(f"{model}: {path}")
