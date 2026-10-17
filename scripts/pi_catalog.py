"""Plan rollback-safe updates to repository-owned Pi movement and cue assets."""
import json
from pathlib import Path
import subprocess


def built_in_yaml(path):
    path = Path(path)
    if path.is_absolute() or '..' in path.parts or path.suffix not in ('.yaml', '.yml'):
        return False
    parts = path.parts
    if 'user' in parts:
        return False
    return (parts[:2] == ('motion', 'config') or
            (parts[:2] == ('motion', 'motions') and len(parts) > 2 and parts[2] != 'user') or
            (parts[:1] == ('scenes',) and len(parts) > 1 and parts[1] != 'user'))


def built_in_asset(path):
    path = Path(path)
    if built_in_yaml(path):
        return True
    return (not path.is_absolute() and '..' not in path.parts and 'user' not in path.parts
            and path.parts[:2] == ('audio', 'cues') and len(path.parts) > 2)


def require_regular_target(project, path):
    for part in (path, *path.parents):
        if part.is_symlink():
            raise ValueError(f'Inspect symlinked catalog asset before deployment: {part}')
        if part == project:
            break
    if path.exists() and not path.is_file():
        raise ValueError(f'Catalog asset is not a file: {path}')


def catalog_plan(release, project, root):
    """Replace built-ins using the rollback transaction; preserve user assets."""
    incoming = {}
    for directory in ('motion/config', 'motion/motions', 'scenes', 'audio/cues'):
        source = release / directory
        if not source.is_dir() or source.is_symlink():
            if directory == 'audio/cues' and not (release / 'asset-manifest.json').exists():
                continue  # Legacy app releases did not manage audio through the catalog.
            raise ValueError(f'Release is missing a regular built-in catalog: {source}')
        for path in sorted(source.rglob('*')):
            relative = path.relative_to(release)
            if built_in_asset(relative):
                require_regular_target(release, path)
                incoming[relative.as_posix()] = path.read_bytes() if relative.parts[:2] == ('audio', 'cues') else path.read_text()
    if 'motion/config/v1/poses.yaml' not in incoming:
        raise ValueError('Release is missing the built-in pose library')
    manifest = root / 'catalog-assets.json'
    if manifest.is_symlink():
        raise ValueError(f'Inspect symlinked catalog inventory: {manifest}')
    if manifest.exists():
        previous = json.loads(manifest.read_text())
        if previous['project'] != str(project):
            raise ValueError('Catalog root changed; inspect the previous asset inventory first')
        old = previous['paths']
        if not isinstance(old, list) or any(not isinstance(p, str) or not built_in_asset(p) for p in old):
            raise ValueError('Invalid built-in catalog inventory')
    else:
        old = []
        # Only migrate a checkout; fresh installs need neither Git nor an index.
        if (project / '.git').exists():
            try:
                tracked = subprocess.run(['git', '-C', str(project), 'ls-files', '-z'],
                                         capture_output=True, text=True)
            except FileNotFoundError:
                pass
            else:
                if tracked.returncode == 0:
                    old = [p for p in tracked.stdout.split('\0') if built_in_asset(p)]
    files = {}
    for relative in sorted(set(old) | incoming.keys()):
        path = project / relative
        require_regular_target(project, path)
        files[path] = incoming.get(relative)  # None removes a retired built-in.
    metadata_path = release / 'asset-manifest.json'
    metadata = json.loads((metadata_path if metadata_path.exists() else release / 'release.json').read_text())
    files[manifest] = json.dumps({'project': str(project), 'revision': metadata.get('revision', metadata.get('version')),
                                 'paths': sorted(incoming)}, indent=2) + '\n'
    return files
