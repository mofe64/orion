"""Select the release boundary from the complete event change set."""
import argparse
import json
from pathlib import Path
import re
import subprocess

SEMVER = r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)'


def asset_path(path):
    parts = Path(path).parts
    return 'user' not in parts and path.startswith(('motion/config/', 'motion/motions/', 'scenes/', 'audio/cues/'))


def classify(paths):
    return 'assets' if any(asset_path(p) for p in paths) and all(asset_path(p) or p == 'ASSETS_VERSION' for p in paths) else 'app'


def version(root, kind):
    value = (root / ('ASSETS_VERSION' if kind == 'assets' else 'VERSION')).read_text().strip()
    if not re.fullmatch(SEMVER, value):
        raise ValueError('Version files must contain X.Y.Z without a prefix')
    return ('assets-v' if kind == 'assets' else 'v') + value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--event', type=Path, required=True)
    parser.add_argument('--event-name', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    event = json.loads(args.event.read_text())
    if args.event_name == 'workflow_dispatch':
        kind = 'app'; tag = event['inputs']['version']
        if not re.fullmatch('v' + SEMVER + r'-[0-9A-Za-z][0-9A-Za-z.-]*', tag):
            raise ValueError('Manual runs require a prerelease tag such as v0.1.0-test.1')
    else:
        if args.event_name == 'pull_request':
            base = event['pull_request']['base']['sha']; head = event['pull_request']['head']['sha']
        else:
            base = event['before']; head = event['after']
        if set(base) == {'0'}:
            base = head + '^'
        comparison = base + ('...' if args.event_name == 'pull_request' else '..') + head
        paths = subprocess.check_output(['git', 'diff', '--name-only', '--no-renames', '-z', comparison]).decode().split('\0')
        paths = [p for p in paths if p]
        kind = classify(paths); tag = version(Path('.'), kind)
        if kind == 'app' and any(asset_path(p) for p in paths):
            asset_tag = version(Path('.'), 'assets')
            if subprocess.run(['git', 'show-ref', '--verify', '--quiet', 'refs/tags/' + asset_tag]).returncode == 0:
                raise ValueError(f'{asset_tag} already exists; bump ASSETS_VERSION for the changed assets')
    # Existing tags are immutable, including manual prereleases.
    if subprocess.run(['git', 'show-ref', '--verify', '--quiet', 'refs/tags/' + tag]).returncode == 0:
        raise ValueError(f'{tag} already exists; bump the version file')
    with args.output.open('a') as output:
        output.write(f'kind={kind}\ntag={tag}\n')


if __name__ == '__main__': main()
