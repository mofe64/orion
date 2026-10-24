"""Keep container bridge addresses out of the lamp's multicast DNS records."""
from pathlib import Path
import re
import subprocess


def deny_interfaces(text, interfaces):
    lines = text.splitlines(keepends=True)
    start = next((i for i, line in enumerate(lines) if line.strip().lower() == '[server]'), None)
    if start is None:
        return text + ('\n' if text and not text.endswith('\n') else '') + '[server]\ndeny-interfaces=' + ','.join(interfaces) + '\n'
    end = next((i for i in range(start+1, len(lines)) if lines[i].lstrip().startswith('[')), len(lines))
    for i in range(start+1, end):
        match = re.match(r'(\s*deny-interfaces\s*=)([^#;\n]*)(.*)', lines[i].rstrip('\n'))
        if match:
            current = [value.strip() for value in match[2].split(',') if value.strip()]
            missing = [value for value in interfaces if value not in current]
            if not missing:
                return text
            lines[i] = match[1] + ','.join(current + missing) + (' ' if match[3] else '') + match[3] + '\n'
            return ''.join(lines)
    if not lines[start].endswith('\n'):
        lines[start] += '\n'
    lines.insert(start+1, 'deny-interfaces=' + ','.join(interfaces) + '\n')
    return ''.join(lines)


def configure_avahi(run, *, prefix=(), path=Path('/etc/avahi/avahi-daemon.conf'), net=Path('/sys/class/net')):
    if not path.exists():
        return False
    text = path.read_text()
    updated = deny_interfaces(text, ['docker0', *(['hassio'] if (net / 'hassio').exists() else [])])
    if updated == text:
        return False
    run(*prefix, 'tee', path, input=updated, text=True, stdout=subprocess.DEVNULL)
    run(*prefix, 'systemctl', 'restart', 'avahi-daemon')
    return True
