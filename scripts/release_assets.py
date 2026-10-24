"""Build, verify and select independent built-in asset packs."""
import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request

from pi_catalog import built_in_asset, require_regular_target
from pi_release_bundle import sha256

FORMAT_VERSION = 1
REPOSITORY = 'mofe64/orion'
PROFILE = 'hardware/profiles/v2-joints.json'
ASSET_DIRECTORIES = ('motion/config', 'motion/motions', 'scenes', 'audio/cues')


def fetch(url):
    request = urllib.request.Request(url, headers={'User-Agent': 'orion-installer'})
    with urllib.request.urlopen(request, timeout=120) as response:
        return response.read()


def releases():
    # GitHub's /latest can point to a newer asset release. Select app and asset
    # namespaces explicitly, and include older compatible packs through pagination.
    page = 1
    while True:
        entries = json.loads(fetch(f'https://api.github.com/repos/{REPOSITORY}/releases?per_page=100&page={page}'))
        if not entries: return
        for entry in entries:
            if not entry['draft'] and not entry['prerelease']: yield entry
        page += 1


def asset_urls(release):
    return {a['name']: a['browser_download_url'] for a in release['assets']}


def unpack_verified(archive, destination, checksum=None):
    if checksum is not None:
        fields = checksum.decode().split()
        expected = fields[0] if fields else ''
        if not re.fullmatch('[a-f0-9]{64}', expected) or sha256(archive) != expected:
            raise RuntimeError('Asset archive checksum mismatch')
    with tarfile.open(archive) as stream:
        from pathlib import PurePosixPath
        names = set()
        for member in stream.getmembers():
            path = PurePosixPath(member.name)
            if (path.is_absolute() or '..' in path.parts or str(path) != member.name.rstrip('/')
                    or member.name in names or not (member.isfile() or member.isdir())):
                raise RuntimeError(f'Unsafe archive member: {member.name}')
            names.add(member.name)
        stream.extractall(destination, filter='data')


def verify_pack(source, app):
    manifest = json.loads((source / 'asset-manifest.json').read_text())
    if manifest['asset_format_version'] != FORMAT_VERSION:
        raise RuntimeError('Unsupported asset format version')
    if manifest['joint_profile_sha256'] != sha256(app / PROFILE):
        raise RuntimeError('Asset joint-profile mismatch')
    files = manifest['files']
    actual = {p.relative_to(source).as_posix() for p in source.rglob('*') if p.is_file()}
    if actual != set(files) | {'asset-manifest.json'} or any(not built_in_asset(p) for p in files):
        raise RuntimeError('Asset inventory mismatch')
    for relative, digest in files.items():
        if sha256(source / relative) != digest:
            raise RuntimeError(f'Asset checksum mismatch: {relative}')
    return manifest


def compile_catalog(source, app):
    # Use immutable profiles and the compiler in the selected app, never candidate code.
    if sha256(source / PROFILE) != sha256(app / PROFILE):
        raise RuntimeError('Candidate profile differs from the latest app release')
    for hardware in ('v1', 'v2'):
        subprocess.run([str(app / 'runtime/target/release/oriond'), '--validate-catalog',
                        '--catalog', str(source), '--hardware', hardware], check=True)


def build_pack(source, app, output):
    compile_catalog(source, app)
    version = (source / 'ASSETS_VERSION').read_text().strip()
    if not re.fullmatch(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', version):
        raise ValueError('ASSETS_VERSION must contain X.Y.Z')
    paths = sorted(p for directory in ASSET_DIRECTORIES for p in (source / directory).rglob('*')
                   if p.is_file() and built_in_asset(p.relative_to(source)))
    for path in paths: require_regular_target(source, path)
    manifest = {'asset_format_version': FORMAT_VERSION, 'version': version,
                'joint_profile_sha256': sha256(app / PROFILE),
                'files': {p.relative_to(source).as_posix(): sha256(p) for p in paths}}
    output.mkdir(parents=True, exist_ok=True)
    name = f'orion-assets-{version}.tar.gz'
    manifest_path = output / f'{name}.manifest.json'
    manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
    archive = output / name
    with tarfile.open(archive, 'w:gz') as stream:
        stream.add(manifest_path, arcname='asset-manifest.json')
        for path in paths: stream.add(path, arcname=path.relative_to(source).as_posix(), recursive=False)
    (output / f'{name}.sha256').write_text(sha256(archive) + '  ' + name + '\n')
    return archive


def remove_assets(release):
    for directory in ASSET_DIRECTORIES:
        for path in (release / directory).rglob('*'):
            if path.is_file() and built_in_asset(path.relative_to(release)): path.unlink()


def download_app(output):
    release = next((r for r in releases() if re.fullmatch(r'v[0-9]+\.[0-9]+\.[0-9]+', r['tag_name'])), None)
    if release is None: raise RuntimeError('No stable app release is available')
    name = f'orion-{release["tag_name"][1:]}-arm64.tar.gz'
    urls = asset_urls(release)
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as folder:
        archive = Path(folder) / name
        archive.write_bytes(fetch(urls[name]))
        unpack_verified(archive, output, fetch(urls[name + '.sha256']))
    return output / 'release'


def select_pack(app, destination, version=None):
    if version is not None and not re.fullmatch(r'assets-v[0-9]+\.[0-9]+\.[0-9]+', version):
        raise ValueError('Use --version assets-vX.Y.Z')
    candidates = [r for r in releases() if re.fullmatch(r'assets-v[0-9]+\.[0-9]+\.[0-9]+', r['tag_name'])
                  and (version is None or r['tag_name'] == version)]
    minimum = app / 'ASSETS_VERSION'
    if version is None and minimum.is_file():
        wanted = tuple(map(int, minimum.read_text().strip().split('.')))
        candidates = [r for r in candidates if tuple(map(int, r['tag_name'][8:].split('.'))) >= wanted]
    candidates.sort(key=lambda r: tuple(map(int, r['tag_name'][8:].split('.'))), reverse=True)
    for release in candidates:
        name = f'orion-assets-{release["tag_name"][8:]}.tar.gz'
        urls = asset_urls(release)
        manifest = json.loads(fetch(urls[name + '.manifest.json']))
        compatible = manifest['asset_format_version'] == FORMAT_VERSION and manifest['joint_profile_sha256'] == sha256(app / PROFILE)
        if not compatible:
            if version: raise RuntimeError('Selected asset pack is incompatible with the installed app')
            continue
        archive = destination.parent / name
        archive.write_bytes(fetch(urls[name]))
        unpack_verified(archive, destination, fetch(urls[name + '.sha256']))
        verified = verify_pack(destination, app)
        if verified != manifest or verified['version'] != release['tag_name'][8:]:
            raise RuntimeError('Asset release manifest mismatch')
        return destination
    if version: raise RuntimeError('Selected asset release does not exist')
    return None


def prepare_catalog(app, destination, first_install=False, version=None):
    destination.mkdir(parents=True, exist_ok=True)
    selected = select_pack(app, destination, version)
    if selected is not None: return selected
    if version is None and (app / 'default-assets.tar.gz').is_file():
        unpack_verified(app / 'default-assets.tar.gz', destination)
        verify_pack(destination, app)
        return destination
    raise RuntimeError('No compatible asset pack is available')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('build', 'download-app'))
    parser.add_argument('--app', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.action == 'download-app': download_app(args.output.resolve())
    elif args.app: build_pack(Path.cwd(), args.app.resolve(), args.output.resolve())
    else: parser.error('--app is required')


if __name__ == '__main__': main()
