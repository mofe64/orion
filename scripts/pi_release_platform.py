#!/usr/bin/env python3
"""Read the robot platform and select a compatible ARM64 Linux builder."""
import argparse
import json
import platform
from pathlib import Path
import sys


TARGET_PLATFORM = {
    'id': 'ubuntu', 'version_id': '24.04', 'machine': 'aarch64',
    'system_python': '3.12', 'libc': 'glibc-2.39',
}
PLATFORM_KEYS = tuple(TARGET_PLATFORM)


def read_platform():
    operating_system = platform.freedesktop_os_release()
    return {
        'id': operating_system['ID'],
        'version_id': operating_system['VERSION_ID'],
        'machine': platform.machine(),
        'system_python': f'{sys.version_info.major}.{sys.version_info.minor}',
        'libc': '-'.join(platform.libc_ver()),
    }


def builder_image(target):
    if any(target.get(key) != value for key, value in TARGET_PLATFORM.items()):
        raise RuntimeError(f'This builder targets Ubuntu 24.04 ARM64, glibc 2.39 and system Python 3.12; found {target}')
    return 'ubuntu:24.04'


def check_platform(expected, actual):
    if any(expected.get(key) != actual.get(key) for key in PLATFORM_KEYS):
        raise RuntimeError(f'Robot platform changed since preparation: built for {expected}, found {actual}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--probe', action='store_true')
    parser.add_argument('--target', type=Path)
    args = parser.parse_args()
    if args.probe:
        print(json.dumps(read_platform(), sort_keys=True))
    elif args.target:
        print(builder_image(json.loads(args.target.read_text())))
    else:
        parser.error('Select --probe or --target')


if __name__ == '__main__':
    main()
