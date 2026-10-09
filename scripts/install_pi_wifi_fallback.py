#!/usr/bin/env python3
"""Install an opt-in hotspot on Wi-Fi already owned by NetworkManager."""
import argparse
import ipaddress
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import tempfile
import uuid

SOURCE = Path(__file__).resolve().parent
CONFIG = Path('/etc/orion/wifi-fallback.env')
HELPER = Path('/usr/local/sbin/orion-wifi-fallback')
UNITS = Path('/etc/systemd/system')
PROFILE = 'orion-fallback-hotspot'
TIMER = 'orion-wifi-fallback.timer'
BACKUPS = Path('/var/lib/orion/wifi-fallback-backups')


def run(*arguments, check=True):
    result = subprocess.run(arguments, capture_output=True, text=True,
                            timeout=30, env=dict(os.environ, LC_ALL='C'))
    if check and result.returncode:
        # Never format the full command: profile creation contains the PSK.
        raise RuntimeError(f'{arguments[0]} failed: {result.stderr.strip()}')
    return result


def setting(profile, field):
    return run('nmcli', '-g', field, 'connection', 'show', 'uuid', profile).stdout.strip()


def saved_config():
    if not CONFIG.exists():
        return None
    if CONFIG.is_symlink():
        raise RuntimeError(f'Refusing symlinked configuration: {CONFIG}')
    return dict(line.split('=', 1) for line in CONFIG.read_text().splitlines() if line)


def preflight(interface, ssid, country):
    if not re.fullmatch(r'[A-Za-z0-9_.-]{1,15}', interface):
        raise ValueError('Invalid Wi-Fi interface')
    if not 1 <= len(ssid.encode()) <= 32 or any(ord(char) < 32 or ord(char) == 127 for char in ssid):
        raise ValueError('SSID must contain 1–32 UTF-8 bytes and no control characters')
    if not re.fullmatch(r'[A-Z]{2}', country):
        raise ValueError('Use a two-letter Wi-Fi country code')
    if not re.search(rf'\bcountry {country}:', run('iw', 'reg', 'get').stdout):
        raise RuntimeError(f'Wi-Fi regulatory country is not {country}; configure it before installing')
    if run('dpkg-query', '-W', '-f=${Status}', 'dnsmasq-base').stdout.strip() != 'install ok installed':
        raise RuntimeError('Install dnsmasq-base before installing the hotspot')
    run('systemctl', 'is-active', '--quiet', 'NetworkManager')
    if run('nmcli', '-g', 'WIFI-PROPERTIES.AP', 'device', 'show', interface).stdout.strip() != 'yes':
        raise RuntimeError(f'{interface} does not report access-point support')
    state = run('nmcli', '-g', 'GENERAL.STATE', 'device', 'show', interface).stdout
    if not re.match(r'^100(?:\s|$)', state):
        raise RuntimeError('Install while normal Wi-Fi is connected; leave active network switching to a separate operation')
    home = str(uuid.UUID(run('nmcli', '-g', 'GENERAL.CON-UUID', 'device', 'show', interface).stdout.strip()))
    if setting(home, '802-11-wireless.mode') != 'infrastructure':
        raise RuntimeError('The active connection must be a normal Wi-Fi client profile')
    if setting(home, 'connection.autoconnect') != 'yes':
        raise RuntimeError('Home Wi-Fi must already be configured to autoconnect on boot')
    addresses = json.loads(run('ip', '-j', '-4', 'address', 'show').stdout)
    subnet = ipaddress.ip_network('10.42.0.0/24')
    for device in addresses:
        for address in device.get('addr_info', []):
            network = ipaddress.ip_interface(f"{address['local']}/{address['prefixlen']}").network
            if network.overlaps(subnet):
                raise RuntimeError(f'Hotspot subnet conflicts with {device["ifname"]}: {network}')
    existing = run('nmcli', '-g', 'connection.uuid', 'connection', 'show', 'id', PROFILE, check=False)
    config = saved_config()
    if existing.returncode == 0:
        hotspot = str(uuid.UUID(existing.stdout.strip()))
        expected = {'ORION_WIFI_INTERFACE': interface, 'ORION_WIFI_HOME_UUID': home,
                    'ORION_WIFI_HOTSPOT_UUID': hotspot}
        if config != expected:
            raise RuntimeError('Existing hotspot/configuration is not this installation; refusing to overwrite it')
        for field, value in {'802-11-wireless.ssid': ssid, '802-11-wireless.mode': 'ap',
                             'connection.interface-name': interface, 'connection.autoconnect': 'no',
                             'ipv4.method': 'shared', 'ipv4.addresses': '10.42.0.1/24',
                             'ipv6.method': 'ignore', '802-11-wireless-security.key-mgmt': 'wpa-psk'}.items():
            if setting(hotspot, field) != value:
                raise RuntimeError(f'Existing hotspot has unexpected {field}; refusing to change it')
        password = None
    elif config is not None:
        raise RuntimeError('Installed configuration has no matching hotspot; inspect it before reinstalling')
    else:
        hotspot = str(uuid.uuid4())
        password = secrets.token_hex(8)
    files = {HELPER: (SOURCE / 'orion_wifi_fallback.py', 0o755)}
    for name in ('orion-wifi-fallback.service', TIMER):
        files[UNITS / name] = (SOURCE / 'systemd' / f'{name}.in', 0o644)
    for path in [CONFIG, *files]:
        if path.is_symlink() or (config is None and path.exists()):
            raise RuntimeError(f'Refusing to overwrite unrelated file: {path}')
    for source, _ in files.values():
        if not source.is_file():
            raise RuntimeError(f'Missing installer source: {source}')
    return home, hotspot, password, files


def write_atomic(path, data, mode):
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix=f'.{path.name}.', dir=path.parent)
    try:
        with os.fdopen(descriptor, 'wb') as output:
            output.write(data)
            os.fchmod(output.fileno(), mode)
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def install(interface, ssid, home, hotspot, password, files):
    BACKUPS.mkdir(parents=True, exist_ok=True)
    backup = Path(tempfile.mkdtemp(prefix='install-', dir=BACKUPS))
    paths = [CONFIG, *files]
    for path in paths:
        if path.exists():
            destination = backup / path.relative_to('/')
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, destination)
    enabled = run('systemctl', 'is-enabled', TIMER, check=False).returncode == 0
    active = run('systemctl', 'is-active', '--quiet', TIMER, check=False).returncode == 0
    created = False
    try:
        if password is not None:
            # A timeout can follow successful creation. Recover by the UUID we
            # reserved, even when the client did not receive the acknowledgement.
            created = True
            run('nmcli', 'connection', 'add', 'type', 'wifi', 'ifname', interface,
                'con-name', PROFILE, 'connection.uuid', hotspot, 'ssid', ssid,
                'connection.autoconnect', 'no', '802-11-wireless.mode', 'ap',
                '802-11-wireless.band', 'bg', 'ipv4.method', 'shared',
                'ipv4.addresses', '10.42.0.1/24', 'ipv6.method', 'ignore',
                'wifi-sec.key-mgmt', 'wpa-psk', 'wifi-sec.proto', 'rsn',
                'wifi-sec.pairwise', 'ccmp', 'wifi-sec.group', 'ccmp',
                'wifi-sec.psk', password)
        values = {'ORION_WIFI_INTERFACE': interface, 'ORION_WIFI_HOME_UUID': home,
                  'ORION_WIFI_HOTSPOT_UUID': hotspot}
        write_atomic(CONFIG, ''.join(f'{key}={value}\n' for key, value in values.items()).encode(), 0o600)
        for path, (source, mode) in files.items():
            write_atomic(path, source.read_bytes(), mode)
        run('systemctl', 'daemon-reload')
        run('systemctl', 'enable', '--now', TIMER)
        print(f'Installed fallback hotspot {ssid}; home profile: {setting(home, "connection.id")}')
        print(f'Studio: http://10.42.0.1:7447; installation backup: {backup}')
        if password is not None:
            print(f'Hotspot password: {password}')
        else:
            print('Existing hotspot password preserved.')
    except BaseException:
        run('systemctl', 'disable', '--now', TIMER, check=False)
        for path in paths:
            original = backup / path.relative_to('/')
            if original.exists():
                write_atomic(path, original.read_bytes(), original.stat().st_mode & 0o777)
            else:
                path.unlink(missing_ok=True)
        run('systemctl', 'daemon-reload', check=False)
        if enabled:
            run('systemctl', 'enable', TIMER, check=False)
        if active:
            run('systemctl', 'start', TIMER, check=False)
        if created:
            current = run('nmcli', '-g', 'GENERAL.CON-UUID', 'device', 'show', interface, check=False).stdout.strip()
            if current != hotspot:
                run('nmcli', 'connection', 'delete', 'uuid', hotspot, check=False)
            else:
                print('Hotspot is active; preserving it for access. Restore normal Wi-Fi before removing the profile.')
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--interface', default='wlan0')
    parser.add_argument('--ssid', default='Orion-Ariadne')
    parser.add_argument('--country', required=True, help='Expected configured two-letter regulatory country')
    parser.add_argument('--plan', action='store_true', help='Inspect prerequisites and print planned paths without writes')
    args = parser.parse_args()
    if not args.plan and os.geteuid() != 0:
        parser.error('Run this installer with sudo, or inspect with --plan')
    try:
        home, hotspot, password, files = preflight(args.interface, args.ssid, args.country)
        if args.plan:
            print(f'Home UUID: {home}; hotspot SSID: {args.ssid}; hotspot autoconnect: no')
            print('\n'.join(str(path) for path in [CONFIG, *files]))
            print('Enable orion-wifi-fallback.timer; do not activate or replace the current Wi-Fi connection.')
        else:
            install(args.interface, args.ssid, home, hotspot, password, files)
    except (ValueError, RuntimeError, OSError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f'{error}\n')


if __name__ == '__main__':
    main()
