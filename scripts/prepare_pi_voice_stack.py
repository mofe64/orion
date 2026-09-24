#!/usr/bin/env python3
"""Download pinned Pi assets and prepare environments without changing services."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request

SILERO_REVISION = '41f03a954b841327835dea1ddb7bb28ae23ddc2c'
CODEX_VERSION = '0.154.0'
CODEX_SHA256 = '97d93e11df72d3c26772db019e6ea8bb72c246500d46b98c760839f3240355e6'
LLAMA_VERSION = 'b10976'
LLAMA_SHA256 = 'ed41c5fd09ae86dbe13e525a57549f1d27d51e7efdc9a9aa4583d42016be7786'
PIPER_ALBA_ARCHIVE = 'vits-piper-en_GB-alba-medium.tar.bz2'
PIPER_ALBA_SHA256 = 'fcd45962906933eec4431d3688f7d74aaac8713c87c6717f91fd3b23463aa1a1'
PIPER_ALBA_WEIGHTS_SHA256 = 'c904d007a8047ab13628b021351b983d0a2627c0d7a81c64a6fe9ad661adb1cf'
PIPER_ALBA_TOKENS_SHA256 = '87c8ef66eae5473ed0cc0366b3964c736ca6c5f676c979522ea31234e47430b9'
QWEN_BASE = 'https://huggingface.co/ggml-org/Qwen3-ASR-0.6B-GGUF/resolve/main/'
MODEL_ASSETS = [
    (QWEN_BASE + 'Qwen3-ASR-0.6B-Q8_0.gguf', 'models/qwen/model.gguf',
     'bca259818b50ca7c4c05e9bdb35a5dc04fa039653a6d6f3f0f331f96f6aa1971'),
    (QWEN_BASE + 'mmproj-Qwen3-ASR-0.6B-Q8_0.gguf', 'models/qwen/mmproj.gguf',
     '41a342b5e4c514e968cb756de6cd1b7be39eff43c44c57a2ef5fc6522e36603d'),
    (f'https://raw.githubusercontent.com/snakers4/silero-vad/{SILERO_REVISION}/src/silero_vad/data/silero_vad.onnx',
     'models/silero.onnx', '1a153a22f4509e292a94e67d6f9b85e8deb25b4988682b7e174c65279d8788e3'),
]


def sha256(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def record_download(root, entry):
    path = root / 'downloads.json'
    entries = json.loads(path.read_text()) if path.exists() else []
    entries = [old for old in entries if old.get('path') != entry['path']]
    entries.append(entry)
    temporary = path.with_suffix('.tmp')
    temporary.write_text(json.dumps(entries, indent=2))
    temporary.replace(path)


def download(root, url, relative, digest):
    """Verify reused files and publish new downloads only after checksum validation."""
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        if sha256(path) != digest:
            raise RuntimeError(f'Checksum mismatch for {path}; existing file was left unchanged')
    else:
        temporary = path.with_name(path.name + '.download')
        try:
            with urllib.request.urlopen(url, timeout=120) as response, temporary.open('wb') as output:
                shutil.copyfileobj(response, output, length=1024 * 1024)
            if sha256(temporary) != digest:
                raise RuntimeError(f'Checksum mismatch for {relative}')
            temporary.replace(path)
        finally:
            temporary.unlink(missing_ok=True)
    record_download(root, dict(url=url, path=str(path), bytes=path.stat().st_size, sha256=digest))
    return path


def prepare_archive(root, name, url, digest, executable, nested=False):
    target = root / name
    if (target / executable).is_file():
        return
    if target.exists():
        raise RuntimeError(f'Incomplete installation at {target}; inspect it before retrying')
    archive = download(root, url, f'downloads/{name}.tar.gz', digest)
    # Never leave a partial target which a retry could mistake for an installation.
    with tempfile.TemporaryDirectory(dir=root, prefix='.extract-') as temporary:
        staging = Path(temporary)
        with tarfile.open(archive) as package:
            package.extractall(staging, filter='data')
        source = staging / name if nested else staging
        if not (source / executable).is_file():
            raise RuntimeError(f'Archive does not contain {executable}')
        source.rename(target)


def prepare_piper_alba(root):
    """Publish the pinned Piper folder only after verifying its archive and weights."""
    target = root / 'models/piper-alba-medium'
    archive = download(root,
        f'https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/{PIPER_ALBA_ARCHIVE}',
        f'downloads/{PIPER_ALBA_ARCHIVE}', PIPER_ALBA_SHA256)

    def validate(folder):
        weights, tokens, data = folder / 'en_GB-alba-medium.onnx', folder / 'tokens.txt', folder / 'espeak-ng-data'
        if not weights.is_file() or not tokens.is_file() or not data.is_dir():
            raise RuntimeError(f'Incomplete Piper Alba model at {folder}')
        if sha256(weights) != PIPER_ALBA_WEIGHTS_SHA256 or sha256(tokens) != PIPER_ALBA_TOKENS_SHA256:
            raise RuntimeError(f'Piper Alba model checksum mismatch at {folder}')

    if target.is_symlink():
        raise RuntimeError(f'Refusing symlinked Piper Alba model: {target}')
    if target.exists():
        validate(target)
        return
    target.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=root, prefix='.extract-piper-') as temporary:
        staging = Path(temporary)
        with tarfile.open(archive, 'r:bz2') as package:
            package.extractall(staging, filter='data')
        source = staging / 'vits-piper-en_GB-alba-medium'
        validate(source)
        source.rename(target)


def prepare_assets(root):
    root.mkdir(parents=True, exist_ok=True)
    prepare_archive(root, f'codex-{CODEX_VERSION}',
        f'https://github.com/openai/codex/releases/download/rust-v{CODEX_VERSION}/codex-package-aarch64-unknown-linux-musl.tar.gz',
        CODEX_SHA256, 'bin/codex')
    prepare_archive(root, f'llama-{LLAMA_VERSION}',
        f'https://github.com/ggml-org/llama.cpp/releases/download/{LLAMA_VERSION}/llama-{LLAMA_VERSION}-bin-ubuntu-arm64.tar.gz',
        LLAMA_SHA256, 'llama-server', nested=True)
    for url, relative, digest in MODEL_ASSETS:
        download(root, url, relative, digest)
    prepare_piper_alba(root)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path.home() / '.local/share/orion/voice-stack')
    args = parser.parse_args()
    root = args.root.resolve()
    prepare_assets(root)
    project = Path(__file__).resolve().parents[1]
    uv = shutil.which('uv') or str(Path.home() / '.local/bin/uv')
    for component, python, extras in [('speech', '3.11', ['--extra', 'pi']), ('voice', '3.12', [])]:
        environment = dict(os.environ, UV_PROJECT_ENVIRONMENT=str(project / component / '.venv'))
        subprocess.run([uv, 'sync', '--project', str(project / component), '--python', python,
                        '--locked', *extras], env=environment, check=True)
        with (root / f'{component}-packages.txt').open('w') as output:
            subprocess.run([uv, 'pip', 'freeze', '--python', str(project / component / '.venv/bin/python')],
                           check=True, stdout=output)
    # Pocket 3.1.0 pins both its weights and preset embeddings to upstream revisions.
    # Preparing named presets uses safetensors conditioning, not reference recordings.
    environment = dict(os.environ, HF_HOME=str(root / 'cache/hf'), HF_HUB_OFFLINE='0',
                       ORION_SPEECH_BACKEND='pi', PYTHONPATH=str(project / 'speech'), ORION_TTS_THREADS='2')
    subprocess.run([str(project / 'speech/.venv/bin/python'), '-c',
        "from orion_speech_worker.pi import PocketSynthesizer,VOICES; "
        "m=PocketSynthesizer('pocket-fp32'); "
        "[(m.model.get_state_for_audio_prompt(v),print('Prepared '+v,flush=True)) for v in VOICES]"],
        env=environment, check=True)
    record_download(root, dict(
        path=str(root / 'cache/hf/hub/models--kyutai--pocket-tts-without-voice-cloning'),
        repo='kyutai/pocket-tts-without-voice-cloning', package='pocket-tts==3.1.0',
        model_revision='d29db7978e464fb90cb3359ee0c69a273b9142cc',
        preset_revision='e81d79e8194ad4c7ce879c87a4258ef20cbf2487'))
    environment['ORION_PIPER_MODEL_DIR'] = str(root / 'models/piper-alba-medium')
    environment['ORION_TTS_THREADS'] = '3'
    subprocess.run([str(project / 'speech/.venv/bin/python'), '-c',
        "from orion_speech_worker.piper import PiperAlbaSynthesizer; "
        "audio=list(PiperAlbaSynthesizer('piper-alba-medium').stream('Orion is ready.')); "
        "assert audio and all(chunk.sample_rate == 24000 for chunk in audio)"],
        env=environment, check=True)
    entries = []
    for folder in [root / 'models', root / 'cache/hf']:
        for path in folder.rglob('*'):
            if path.is_file() and not path.is_symlink():
                entries.append(dict(path=str(path), bytes=path.stat().st_size))
    (root / 'model-files.json').write_text(json.dumps(entries, indent=2))
    print('Pi speech assets prepared; running services were not changed.')


if __name__ == '__main__':
    main()
