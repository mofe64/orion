#!/usr/bin/env python3
"""Send librespot's pipe PCM to oriond. This service never opens ALSA."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import select
import socket
import subprocess
import time


def command(binary, name, cache):
    return [str(binary), '--name', name, '--backend', 'pipe', '--format', 'S16',
        '--volume-ctrl', 'fixed', '--initial-volume', '100', '--device-type', 'speaker',
        '--system-cache', str(cache), '--disable-audio-cache']


def read_settings(path):
    try: value = json.loads(path.read_text())
    except (OSError, ValueError): return {}
    return value if isinstance(value, dict) else {}


def environment(settings):
    result = os.environ.copy()
    result.pop('LIBRESPOT_ACCESS_TOKEN', None)
    # Bootstrap Connect after Studio sign-in. Librespot stores its own session
    # credentials on the lamp; the Web API refresh token never leaves the lamp.
    if settings.get('access_token') and settings.get('expires_at', 0) > time.time()+30:
        result['LIBRESPOT_ACCESS_TOKEN'] = settings['access_token']
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--socket', type=Path, default=Path.home() / '.local/share/orion/music.sock')
    args = parser.parse_args()
    expected = args.binary.with_suffix('.sha256').read_text().split()[0]
    if hashlib.sha256(args.binary.read_bytes()).hexdigest() != expected:
        raise RuntimeError('The installed Spotify receiver checksum does not match. Update Orion to restore it.')
    settings_file = Path.home() / '.config/orion/spotify.json'
    settings = read_settings(settings_file)
    name = settings.get('device_name') or os.environ.get('ORION_NAME') or 'Orion'
    cache = Path.home() / '.local/share/orion/spotify'
    cache.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(cache, 0o700)
    with subprocess.Popen(command(args.binary, name, cache), stdout=subprocess.PIPE, env=environment(settings)) as process:
        def stop(_signal, _frame):
            process.terminate()
            raise SystemExit(0)
        signal.signal(signal.SIGTERM, stop)
        signal.signal(signal.SIGINT, stop)
        try:
            forward_pcm(process, args.socket, settings_file, settings)
        finally:
            if process.poll() is None: process.terminate()
            try: process.wait(timeout=5)
            except subprocess.TimeoutExpired: process.kill(); process.wait()
        if process.returncode: raise SystemExit(process.returncode)


def forward_pcm(process, path, settings_file, settings):
    output, retry_at, backoff = None, 0., 0.1
    position = 0
    pending = b""
    align = 0
    try:
        while process.poll() is None:
            if output is None and time.monotonic() >= retry_at:
                candidate = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                candidate.settimeout(0.25)
                candidate.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, 4096)
                try:
                    candidate.connect(str(path))
                    output, backoff = candidate, 0.1
                    pending = b""
                    align = (-position) % 4
                except OSError:
                    candidate.close()
                    retry_at = time.monotonic() + backoff
                    backoff = min(backoff * 2, 2.)
            ready, _, _ = select.select([process.stdout], [], [], 0.1)
            if ready:
                data = process.stdout.read1(4096)
                if not data: break
                position += len(data)
                # Drain PCM during an outage so Connect remains responsive and
                # reconnect starts with current audio rather than an old song.
                if output is not None:
                    skip = min(align, len(data))
                    align -= skip
                    pending += data[skip:]
                    whole = len(pending) // 4 * 4
                    frame_data, pending = pending[:whole], pending[whole:]
                    try:
                        if frame_data:
                            output.sendall(frame_data)
                    except OSError:
                        output.close()
                        output = None
                        pending = b""
                        retry_at = time.monotonic() + backoff
                        backoff = min(backoff * 2, 2.)
            if read_settings(settings_file).get('connect_revision') != settings.get('connect_revision'):
                break
    finally:
        if output is not None: output.close()


if __name__ == '__main__': main()
