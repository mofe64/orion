"""Plan rollback-safe updates to repository-owned Pi YAML assets."""
import json
from pathlib import Path
import subprocess


def built_in_yaml(path):
    path = Path(path)
    if path.is_absolute() or '..' in path.parts or path.suffix not in ('.yaml', '.yml'):
        return False
    parts = path.parts
    return ((parts[:2] == ('hardware', 'v2') and 'user' not in parts) or
            parts[:2] == ('motion', 'config') or
            (parts[:2] == ('motion', 'motions') and len(parts) > 2 and parts[2] != 'user') or
            (parts[:1] == ('scenes',) and len(parts) > 1 and parts[1] != 'user'))


def require_regular_target(project, path):
    for part in (path, *path.parents):
        if part.is_symlink():
            raise ValueError(f'Inspect symlinked catalog asset before deployment: {part}')
        if part == project:
            break
    if path.exists() and not path.is_file():
        raise ValueError(f'Catalog asset is not a file: {path}')


def catalog_plan(release, project, root):
    """Replace built-ins and remove retired tracked YAML; leave user assets alone."""
    incoming = {}
    for directory in ('motion/config', 'motion/motions', 'scenes', 'hardware/v2'):
        source = release / directory
        if directory == 'hardware/v2' and not source.exists():
            continue  # Legacy v1 release.
        if not source.is_dir() or source.is_symlink():
            raise ValueError(f'Release is missing a regular built-in catalog: {source}')
        for path in sorted(source.rglob('*')):
            relative = path.relative_to(release)
            if built_in_yaml(relative):
                require_regular_target(release, path)
                incoming[relative.as_posix()] = path.read_text()
    if 'motion/config/poses.yaml' not in incoming:
        raise ValueError('Release is missing the built-in pose library')
    manifest = root / 'catalog-assets.json'
    if manifest.is_symlink():
        raise ValueError(f'Inspect symlinked catalog inventory: {manifest}')
    if manifest.exists():
        previous = json.loads(manifest.read_text())
        if previous['project'] != str(project):
            raise ValueError('Catalog root changed; inspect the previous asset inventory first')
        old = previous['paths']
        if not isinstance(old, list) or any(not isinstance(p, str) or not built_in_yaml(p) for p in old):
            raise ValueError('Invalid built-in catalog inventory')
    else:
        # Migrate the existing Git catalog without changing HEAD or the index.
        tracked = subprocess.run(['git', '-C', str(project), 'ls-files', '-z'],
                                 capture_output=True, text=True)
        old = [p for p in tracked.stdout.split('\0') if built_in_yaml(p)] if tracked.returncode == 0 else []
    files = {}
    for relative in sorted(set(old) | incoming.keys()):
        path = project / relative
        require_regular_target(project, path)
        files[path] = incoming.get(relative)  # None removes a retired built-in.
    metadata = json.loads((release / 'release.json').read_text())
    files[manifest] = json.dumps({'project': str(project), 'revision': metadata['revision'],
                                 'paths': sorted(incoming)}, indent=2) + '\n'
    return files
