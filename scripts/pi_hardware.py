"""Hardware selection and calibration paths; no hardware writes."""
import json
from pathlib import Path

PROJECT = Path(__file__).resolve().parents[1]


def profile(hardware, project=PROJECT):
    if hardware not in ('v1', 'v2'):
        raise ValueError('Select hardware v1 or v2')
    selected = json.loads((project / 'hardware/profiles' / f'{hardware}.json').read_text())
    if hardware == 'v2':
        joints = json.loads((project / selected['joint_profile']).read_text())['joints']
        for name, joint in selected['joints'].items(): joint['servo_id'] = joints[name]['servo_id']
    return selected


def release_hardware(release):
    # Releases prepared before hardware selection were exclusively v1.
    return json.loads((release / 'release.json').read_text()).get('hardware', 'v1')


def calibration_path(home, hardware, project=PROJECT):
    return project / profile(hardware)['joint_profile'] if hardware == 'v2' else home / '.config/orion' / profile(hardware)['calibration_file']

