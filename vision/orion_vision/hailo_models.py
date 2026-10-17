"""Optional device discovery and architecture-specific, pinned model files.

This module uses only the standard library so the release installer can use it
before component environments exist. Discovery never opens a VDevice.
"""
import hashlib
import json
from pathlib import Path
import re
import subprocess

MODEL_DIRECTORY = Path('/usr/local/share/orion/vision/hailo')


def model_manifest():
    return json.loads(Path(__file__).with_name('hailo-models.json').read_text())


def device_kind(output):
    architecture = re.search(r'^[ \t]*Device Architecture:[ \t]*([^\r\n]*)', output, re.I | re.M)
    if architecture is None:
        raise RuntimeError('Missing Device Architecture line in hailortcli identify output')
    value = architecture[1].strip()
    match = re.fullmatch(r'HAILO[-_ ]?(8L|8)', value, re.I)
    if match is None:
        raise RuntimeError(f'Unsupported Device Architecture: {value}')
    return 'HAILO' + match[1].upper()


def device_architecture():
    try:
        pci = subprocess.run(['lspci', '-Dn'], capture_output=True, text=True, timeout=5, check=True)
    except (OSError, subprocess.SubprocessError):
        return None
    if '1e60:' not in pci.stdout.lower():
        return None
    result = subprocess.run(['hailortcli', 'fw-control', 'identify'], capture_output=True,
                            text=True, timeout=10, check=True)
    return device_kind(result.stdout)


def model_paths(architecture, directory=MODEL_DIRECTORY):
    return {name: Path(directory) / entry['path']
            for name, entry in model_manifest()[architecture].items()}


def checked_model_paths(architecture, directory=MODEL_DIRECTORY):
    paths = model_paths(architecture, directory)
    for name, path in paths.items():
        with path.open('rb') as source:
            digest = hashlib.file_digest(source, 'sha256').hexdigest()
        if digest != model_manifest()[architecture][name]['sha256']:
            raise ValueError(f'{name} HEF SHA-256 mismatch: {path}')
    return paths
