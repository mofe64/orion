#!/usr/bin/python3
"""Recover an idle NetworkManager Wi-Fi device without replacing a live link."""
import os
from pathlib import Path
import re
import subprocess
import sys
import uuid


BUSY = {40, 50, 60, 70, 80, 90, 110}
IDLE = {30, 120}


def nmcli(*arguments):
    # Bound even a wedged D-Bus client; two activations plus state reads fit 50s.
    timeout = 21 if '--wait' in arguments else 2
    return subprocess.run(['nmcli', *arguments], capture_output=True, text=True,
                          timeout=timeout, env=dict(os.environ, LC_ALL='C'))


def device_state(interface, run):
    result = run('-g', 'GENERAL.STATE', 'device', 'show', interface)
    match = re.fullmatch(r'(\d+)(?:\s+\([^\n]*\))?\s*', result.stdout)
    if result.returncode or not match:
        raise RuntimeError(f'Cannot read NetworkManager state for {interface}')
    return int(match[1])


def recover(interface, home, hotspot, run=nmcli, log=print, attempt_file=None):
    # Keep an unfinished home attempt across timer runs. A CLI timeout leaves
    # NetworkManager working; retrying home after each failure starves the AP.
    attempt = f'{interface}:{home}\n'

    def forget_attempt():
        if attempt_file is not None:
            attempt_file.unlink(missing_ok=True)

    state = device_state(interface, run)
    if state == 100:
        forget_attempt()
        return 0
    if state in BUSY:
        return 0
    if state not in IDLE:
        raise RuntimeError(f'{interface} is unavailable or unmanaged (state {state}); leaving it alone')

    pending = (attempt_file is not None and attempt_file.exists()
               and attempt_file.read_text() == attempt)
    if not pending:
        if attempt_file is not None:
            temporary = attempt_file.with_suffix('.tmp')
            temporary.write_text(attempt)
            temporary.replace(attempt_file)
        try:
            result = run('--wait', '20', 'connection', 'up', 'uuid', home, 'ifname', interface)
        except subprocess.TimeoutExpired:
            # The client timing out does not establish that NM has stopped trying.
            result = None
        if result is not None and result.returncode == 0:
            forget_attempt()
            log('Home Wi-Fi reconnected')
            return 0

    state = device_state(interface, run)
    if state == 100:
        forget_attempt()
        return 0
    if state in BUSY:
        log('Home attempt still in progress; waiting for NetworkManager')
        return 0
    if state not in IDLE:
        raise RuntimeError(f'{interface} changed to state {state}; leaving it alone')
    result = run('--wait', '20', 'connection', 'up', 'uuid', hotspot, 'ifname', interface)
    if result.returncode:
        raise RuntimeError('Could not activate fallback hotspot; inspect NetworkManager journal')
    forget_attempt()
    log('Hotspot active at 10.42.0.1')
    return 0


def main():
    try:
        interface = os.environ['ORION_WIFI_INTERFACE']
        if not re.fullmatch(r'[A-Za-z0-9_.-]{1,15}', interface):
            raise ValueError('Invalid Wi-Fi interface')
        home = str(uuid.UUID(os.environ['ORION_WIFI_HOME_UUID']))
        hotspot = str(uuid.UUID(os.environ['ORION_WIFI_HOTSPOT_UUID']))
        if home == hotspot:
            raise ValueError('Home and hotspot profiles must differ')
        return recover(interface, home, hotspot,
                       attempt_file=Path('/run/orion-wifi-fallback/home-attempt'))
    except (KeyError, ValueError, RuntimeError, OSError, subprocess.TimeoutExpired) as error:
        print(f'orion-wifi-fallback: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
