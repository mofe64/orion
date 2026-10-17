"""Checksummed release bundles contain files and wheels, never compiler caches."""
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import subprocess
import tarfile
import zipfile


RESERVE_BYTES = 512 * 1024**2


def sha256(path):
    with Path(path).open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def payload_files(folder):
    return {p.relative_to(folder).as_posix(): p for p in folder.rglob('*') if p.is_file()}


def write_manifest(folder, release, target):
    files = payload_files(folder)
    files.pop('bundle.json', None)
    files.pop('SHA256SUMS', None)
    expanded = 0
    for path in (folder / 'wheels').rglob('*.whl'):
        with zipfile.ZipFile(path) as wheel:
            expanded += sum(member.file_size for member in wheel.infolist())
    metadata = json.loads((release / 'release.json').read_text())
    manifest = {
        'version': 1, 'release_id': metadata['release_id'],
        'commit': metadata['commit'], 'hardware': metadata['hardware'],
        'platform': target, 'expanded_packages_bytes': expanded,
        'files': {name: sha256(path) for name, path in sorted(files.items())},
    }
    (folder / 'bundle.json').write_text(json.dumps(manifest, indent=2) + '\n')
    return manifest


def inspect_archive(archive):
    """Reject traversal, links, duplicate paths and special files before extraction."""
    names = set()
    for member in archive.getmembers():
        path = PurePosixPath(member.name)
        if path.is_absolute() or '..' in path.parts or str(path) != member.name.rstrip('/'):
            raise RuntimeError(f'Unsafe bundle path: {member.name}')
        if member.name in names or not (member.isfile() or member.isdir()):
            raise RuntimeError(f'Unsupported bundle member: {member.name}')
        names.add(member.name)
    member = archive.getmember('bundle.json')
    if not member.isfile() or member.size > 1024**2:
        raise RuntimeError('Invalid bundle manifest')
    manifest = json.load(archive.extractfile(member))
    if manifest.get('version') != 1 or not re.fullmatch(r'[0-9a-f]{12}-[0-9a-f]{8}', manifest.get('release_id', '')):
        raise RuntimeError('Unsupported release bundle')
    if not re.fullmatch(r'[0-9a-f]{40}', manifest.get('commit', '')) or manifest.get('hardware') not in ('v1', 'v2'):
        raise RuntimeError('Invalid release identity')
    if not manifest['release_id'].startswith(manifest['commit'][:12] + '-'):
        raise RuntimeError('Release identity does not match its commit')
    expanded = manifest.get('expanded_packages_bytes')
    if not isinstance(expanded, int) or expanded < 0:
        raise RuntimeError('Invalid package storage estimate')
    required = sum(member.size for member in archive.getmembers()) + expanded + RESERVE_BYTES
    return manifest, required


def verify_payload(folder, manifest):
    files = payload_files(folder)
    files.pop('bundle.json', None)
    files.pop('SHA256SUMS', None)
    if set(files) != set(manifest['files']):
        raise RuntimeError('Release bundle file inventory does not match its manifest')
    for name, path in files.items():
        if sha256(path) != manifest['files'][name]:
            raise RuntimeError(f'Release bundle checksum mismatch: {name}')
    metadata = json.loads((folder / 'release/release.json').read_text())
    if any(metadata.get(key) != manifest[key] for key in ('commit', 'hardware')):
        raise RuntimeError('Release metadata does not match the bundle')


def pack_bundle(folder, destination):
    with tarfile.open(destination, 'w:gz') as archive:
        for path in sorted(folder.rglob('*')):
            if path.is_symlink():
                raise RuntimeError(f'Bundle contains a symlink: {path}')
            archive.add(path, arcname=path.relative_to(folder).as_posix(), recursive=False)


def install_wheels(python, wheels, run):
    packages = sorted(wheels.glob('*.whl'))
    if not packages:
        raise RuntimeError(f'No prepared wheels in {wheels}')
    run('uv', 'pip', 'install', '--python', python, '--no-config', '--offline', '--no-index',
        '--no-build', '--no-deps', '--no-cache', '--link-mode', 'copy', *packages)
    try:
        run('uv', 'pip', 'check', '--python', python, '--no-config', '--offline', '--no-cache',
            capture_output=True, text=True)
    except subprocess.CalledProcessError as error:
        diagnostics = (error.stdout or '') + (error.stderr or '')
        # The pinned NCNN wheel declares desktop OpenCV. The vision lock
        # deliberately supplies the ABI-compatible headless cv2 distribution.
        # Accept exactly this single missing distribution, never other failures.
        expected = "The package `ncnn` requires `opencv-python`, but it's not installed"
        if (not any(p.name.startswith('ncnn-1.0.20250503-') for p in packages)
                or not any(p.name.startswith('opencv_python_headless-4.11.0.86-') for p in packages)
                or 'Found 1 incompatibility\n' not in diagnostics
                or diagnostics.strip().splitlines()[-1] != expected):
            raise
        run(python, '-c', 'import cv2, ncnn; assert cv2.__version__ == "4.11.0"')
