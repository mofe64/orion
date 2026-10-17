"""Explicit AI HAT+ commissioning benchmark; independent of Orion's detectors."""
import argparse
import datetime
import json
from pathlib import Path
import re
import subprocess

from prepare_pi_voice_stack import download

MODELS = {
    'HAILO8': ('hailo8', 'd7adf09a1760be0f444dec7b99d6804e270939d96411a40e60016ed497178668'),
    'HAILO8L': ('hailo8l', '5cf052c3454ddede135dc0baff2c2c0e88611ad376431f797bb8a85c07b4eb14'),
}


def device_kind(output):
    match = re.search(r'HAILO[-_ ]?(8L|8)\b', output, re.I)
    if not match: raise RuntimeError('Expected Hailo-8 or Hailo-8L from hailortcli fw-control identify')
    return 'HAILO' + match[1].upper()


def run(*args):
    result = subprocess.run([str(arg) for arg in args], capture_output=True, text=True, timeout=60)
    print(result.stdout, end='')
    if result.returncode:
        raise RuntimeError(f'{args[0]} failed: {result.stderr.strip() or result.stdout.strip()}')
    return result.stdout


def check(root):
    if '1e60:' not in run('lspci', '-Dn').lower():
        print('AI HAT+: absent; skipping Hailo check.'); return
    identified = run('hailortcli', 'fw-control', 'identify')
    kind = device_kind(identified)
    # Open/release a real device through the vision interpreter, in addition to
    # proving that its C++ binding imports against the installed runtime.
    from hailo_platform import Device
    with Device() as device:
        print(f'Python hailo_platform opened {device}')
    architecture, digest = MODELS[kind]
    model = download(root, f'https://hailo-model-zoo.s3.eu-west-2.amazonaws.com/ModelZoo/Compiled/v2.14.0/{architecture}/resnet_v1_18.hef',
                     f'models/hailo/{architecture}/resnet_v1_18.hef', digest)
    output = run('hailortcli', 'run', model, '--time-to-run', '5')
    match = re.search(r'FPS\s*:\s*([0-9.]+)', output, re.I)
    if not match: raise RuntimeError('Hailo benchmark did not report FPS')
    result = {'device': kind, 'fps': float(match[1]), 'model': str(model), 'sha256': digest,
              'checked_at': datetime.datetime.now(datetime.UTC).isoformat()}
    (root / 'hailo-check.json').write_text(json.dumps(result, indent=2) + '\n')
    print(f'{kind}: {result["fps"]:.2f} FPS; saved {root / "hailo-check.json"}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(); parser.add_argument('--root', type=Path, required=True)
    try:
        check(parser.parse_args().root)
    except Exception as error:
        raise SystemExit(f'Hailo check failed: {error}') from None
