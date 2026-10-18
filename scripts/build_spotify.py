"""Build the checksum-pinned Spotify Connect receiver in the ARM release image."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import urllib.request


def build(root, destination, cache=Path('/cache/librespot')):
    pins = json.loads((root / 'scripts/release-spotify.json').read_text())
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / (pins['sha256'] + '.tar.gz')
    if not archive.exists():
        with urllib.request.urlopen(pins['url'], timeout=60) as response:
            data = response.read()
        if hashlib.sha256(data).hexdigest() != pins['sha256']:
            raise RuntimeError('librespot source checksum does not match release-spotify.json')
        archive.write_bytes(data)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != pins['sha256']:
        raise RuntimeError('Cached librespot source checksum does not match release-spotify.json')
    source = cache / ('source-' + pins['sha256'][:16])
    if not source.exists():
        source.mkdir()
        with tarfile.open(archive) as package:
            package.extractall(source, filter='data')
    project = source / ('librespot-' + pins['version'])
    subprocess.run(['cargo', 'build', '--release', '--locked', '--no-default-features', '--features', ','.join(pins['features'])], cwd=project, check=True)
    destination.mkdir(parents=True, exist_ok=True)
    binary = destination / 'librespot'
    shutil.copy2(Path(os.environ['CARGO_TARGET_DIR']) / 'release/librespot', binary)
    subprocess.run([str(binary), '--version'], check=True)
    (destination / 'librespot.sha256').write_text(hashlib.sha256(binary.read_bytes()).hexdigest() + '  librespot\n')
    shutil.copy2(project / 'LICENSE', destination / 'librespot-LICENSE')


if __name__ == '__main__':
    build(Path('/src'), Path('/out/payload/release/native'))
