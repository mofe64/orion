#!/usr/bin/env python3
"""Raspberry Pi OS compatibility; Python is supplied by the release."""
import argparse
import json
import platform
from pathlib import Path

SUPPORTED_CODENAMES = ('bookworm', 'trixie')
TARGET_PLATFORM = {'id': 'raspbian', 'version_codename': 'bookworm', 'machine': 'aarch64', 'libc': 'glibc-2.36'}
PLATFORM_KEYS = ('machine',)


def read_platform():
    os_release = platform.freedesktop_os_release()
    return {'id': os_release['ID'], 'version_codename': os_release.get('VERSION_CODENAME', ''),
            'machine': platform.machine(), 'libc': '-'.join(platform.libc_ver()),
            'raspberry_pi_os': Path('/etc/rpi-issue').exists()}


def builder_image(target):
    if target.get('machine') not in ('aarch64', 'arm64') or target.get('version_codename') not in SUPPORTED_CODENAMES:
        raise RuntimeError(f'Requires Raspberry Pi OS Bookworm or Trixie ARM64; found {target}')
    # All shared binaries use the oldest supported glibc. Only native binding
    # wheels are rebuilt for the Trixie library ABI.
    return 'debian:bookworm'


def check_platform(expected, actual):
    builder_image(actual)
    if actual.get('id') not in ('debian', 'raspbian') or actual.get('raspberry_pi_os') is False:
        raise RuntimeError('Requires Raspberry Pi OS, identified by /etc/rpi-issue')
    if expected.get('machine') not in ('aarch64', 'arm64'):
        raise RuntimeError('Release must target ARM64')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--probe', action='store_true')
    parser.add_argument('--target', type=Path)
    args = parser.parse_args()
    if args.probe: print(json.dumps(read_platform(), sort_keys=True))
    elif args.target: print(builder_image(json.loads(args.target.read_text())))
    else: parser.error('Select --probe or --target')


if __name__ == '__main__': main()
