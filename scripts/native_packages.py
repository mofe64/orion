"""Resolve the Raspberry Pi package versions required by each native wheel set."""
import argparse
import gzip
import json
from pathlib import Path
import urllib.request

ARCHIVE = 'https://archive.raspberrypi.com/debian/dists/{codename}/main/binary-arm64/Packages.gz'
UNAVAILABLE = "This Orion release's camera/Hailo packages are no longer in the Raspberry Pi archive. Install a newer Orion release."


def pinned_packages(pins, hailo=True, build=False):
    packages = [pins['libcamera_package'] + '=' + pins['libcamera_version'],
                'libcamera-ipa=' + pins['libcamera_version'],
                'libkms++0=' + pins['kms_version'], 'kms++-utils=' + pins['kms_version']]
    if build:
        packages += ['libcamera-dev=' + pins['libcamera_version'], 'libkms++-dev=' + pins['kms_version']]
    return packages + (pins['hailo_packages'] if hailo else [])


def check_archives(manifest, fetch=None):
    def download(url):
        with urllib.request.urlopen(url, timeout=60) as response:
            return response.read()
    fetch = fetch or download
    for codename in ('bookworm', 'trixie'):
        text = gzip.decompress(fetch(ARCHIVE.format(codename=codename))).decode()
        available = set()
        for paragraph in text.split('\n\n'):
            fields = dict(line.split(': ', 1) for line in paragraph.splitlines() if ': ' in line and not line.startswith(' '))
            if 'Package' in fields and 'Version' in fields:
                available.add(fields['Package'] + '=' + fields['Version'])
        missing = sorted(set(pinned_packages(manifest[codename], build=True)) - available)
        if missing:
            raise RuntimeError(f'{UNAVAILABLE}\n{codename}: {", ".join(missing)}')
        print(f'{codename}: all runtime and build package pins resolve.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--check-archives', action='store_true', required=True)
    parser.parse_args()
    try:
        check_archives(json.loads(Path(__file__).with_name('release-native.json').read_text()))
    except (RuntimeError, OSError) as error:
        raise SystemExit(str(error)) from None
