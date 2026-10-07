"""Hardware selection and calibration paths; no hardware writes."""
import json
from pathlib import Path

PROJECT = Path(__file__).resolve().parents[1]


def profile(hardware, project=PROJECT):
    if hardware not in ('v1', 'v2'):
        raise ValueError('Select hardware v1 or v2')
    return json.loads((project / 'hardware/profiles' / f'{hardware}.json').read_text())


def release_hardware(release):
    # Releases prepared before hardware selection were exclusively v1.
    return json.loads((release / 'release.json').read_text()).get('hardware', 'v1')


def calibration_path(home, hardware):
    return home / '.config/orion' / profile(hardware)['calibration_file']

