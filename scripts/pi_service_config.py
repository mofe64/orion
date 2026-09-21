"""Plan a Pi release switch while preserving installed service settings."""
import json
from pathlib import Path
import re
import shlex

SERVICES = ('oriond', 'orion-studio-gateway', 'orion-listener', 'orion-voice-stack')
STOP_ORDER = tuple(reversed(SERVICES))
PATH_SUFFIXES = (
    'runtime/target/release/oriond', 'runtime/target/release/orion-trajectory',
    'orion-service/target/release/orion-service', 'studio-service/target/release/orion-studio-headless',
    'voice/.venv/bin/orion-listener', 'orion_studio/gateway.py',
    'scripts/wait_for_oriond.sh', 'scripts/orion_safe_stop.sh',
)


def quote(value):
    if any(c in str(value) for c in '\n\r\0'):
        raise ValueError('Service values cannot contain line breaks or NUL')
    return '"' + str(value).replace('\\', '\\\\').replace('"', '\\"') + '"'


def read_env(text):
    """Read the single-line systemd assignments we manage; never execute a shell."""
    values = {}
    for line in text.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith(('#', ';')):
            continue
        key, sep, value = stripped.partition('=')
        if not sep or not re.fullmatch(r'[A-Za-z_][A-Za-z_0-9]*', key):
            raise ValueError('Unsupported environment assignment; no configuration was changed')
        parts = shlex.split(value, comments=False)
        # Keep ordinary unquoted values containing spaces, as systemd does.
        values[key] = ' '.join(parts)
    return values


def merge_env(text, defaults, managed):
    """Only release-owned paths change. Existing tuning and comments survive verbatim."""
    existing = read_env(text)
    lines = []
    for line in text.splitlines(keepends=True):
        key = line.strip().partition('=')[0]
        if key in managed:
            lines.append(f'{key}={quote(managed[key])}\n')
        else:
            lines.append(line)
    result = ''.join(lines)
    if result and not result.endswith('\n'):
        result += '\n'
    for key, value in {**defaults, **managed}.items():
        if key not in existing:
            result += f'{key}={quote(value)}\n'
    return result


def rebase_commands(text, release):
    """Repoint executables in base units and overrides without changing their arguments."""
    lines = []
    for line in text.splitlines(keepends=True):
        if re.match(r'^\s*Exec(?:Start|StartPre|Stop)=', line):
            for suffix in PATH_SUFFIXES:
                replacement = 'orion-service/target/release/orion-service' if suffix.startswith('studio-service/') else suffix
                line = re.sub(r'/[A-Za-z0-9._/-]+/' + re.escape(suffix) + r'(?=[\s"\']|$)',
                              lambda _: str(release / replacement), line)
        lines.append(line)
    return ''.join(lines)


def effective_start(contents):
    """Return the last ExecStart after systemd's empty-assignment resets."""
    result = ''
    for _, value in contents:
        for match in re.finditer(r'^ExecStart=(.*)$', value, re.M):
            result = match[1]
    return result


def render_plan(release, root, runtime_project, home, user, unit_dir=Path('/etc/systemd/system')):
    for path in [release, root, runtime_project, home]:
        if not re.fullmatch(r'/[A-Za-z0-9._/-]+', str(path)) or '..' in path.parts:
            raise ValueError(f'Unsupported service path: {path}')
    if not re.fullmatch(r'[A-Za-z_][A-Za-z_0-9-]*', user):
        raise ValueError('Unsupported service user')
    files = {}
    for name in SERVICES:
        path = unit_dir / f'{name}.service'
        if path.is_symlink():
            raise ValueError(f'Inspect symlinked service before deployment: {path}')
        existing = path.exists()
        text = path.read_text() if existing else (release / f'scripts/systemd/{name}.service.in').read_text()
        for key, value in {'PROJECT_ROOT': runtime_project, 'USER_HOME': home, 'ORION_USER': user}.items():
            text = text.replace('@' + key + '@', str(value))
        overrides = sorted((unit_dir / f'{name}.service.d').glob('*.conf'))
        contents = [(path, text)]
        for override in overrides:
            if override.is_symlink():
                raise ValueError(f'Inspect symlinked override before deployment: {override}')
            contents.append((override, override.read_text()))
        combined = '\n'.join(value for _, value in contents)
        # A first onboard installation adds only missing defaults. Existing override values win.
        if name == 'orion-listener':
            start = effective_start(contents)
            if '--local-processor' not in start:
                # Append to the effective (last) ExecStart, preserving custom flags.
                for i in range(len(contents) - 1, -1, -1):
                    p, value = contents[i]
                    if re.search(r'^ExecStart=.+', value, re.M):
                        value = re.sub(r'^(ExecStart=.+)$', r'\1 --local-processor', value, flags=re.M)
                        if '--threshold' not in start:
                            value = re.sub(r'^(ExecStart=.+)$', r'\1 --threshold 0.45', value, flags=re.M)
                        contents[i] = (p, value)
                        break
            additions = []
            if 'ORION_VAD_MODEL=' not in combined:
                additions.append(f'Environment=ORION_VAD_MODEL={root}/models/silero.onnx')
            if 'ORION_CAPTURE_GAIN_DB=' not in combined:
                additions.append('Environment=ORION_CAPTURE_GAIN_DB=25')
            if additions:
                p, value = contents[0]
                contents[0] = (p, value.replace('[Service]', '[Service]\n' + '\n'.join(additions), 1))
        for p, value in contents:
            value = rebase_commands(value, release)
            if name in ('orion-listener', 'orion-voice-stack'):
                directory = release / 'voice' if name == 'orion-listener' else release
                value = re.sub(r'^WorkingDirectory=.*$', f'WorkingDirectory={directory}', value, flags=re.M)
            if name == 'orion-voice-stack':
                value = re.sub(r'^Environment=ORION_PROJECT_ROOT=.*$', f'Environment=ORION_PROJECT_ROOT={release}', value, flags=re.M)
            files[p] = value
    environment_path = home / '.config/orion/voice-stack.env'
    text = environment_path.read_text() if environment_path.exists() else ''
    defaults = {
        'ORION_STUDIO_CODEX_BIN': str(root / 'codex-0.154.0/bin/codex'),
        'ORION_ASR_MODEL_DIR': str(root / 'models/qwen'), 'ORION_LLAMA_SERVER': str(root / 'llama-b10976/llama-server'),
        'ORION_ASR_THREADS': '3', 'ORION_TTS_THREADS': '3', 'HF_HOME': str(root / 'cache/hf'),
        'HF_HUB_OFFLINE': '1', 'ORION_STUDIO_TTS_MODEL': 'pocket-fp32',
    }
    metadata = json.loads((release / 'release.json').read_text()) if (release / 'release.json').exists() else {}
    files[environment_path] = merge_env(text, defaults, {
        'ORION_STUDIO_VOICE_PYTHON': str(release / 'speech/.venv/bin/python'),
        'ORION_RELEASE_REVISION': metadata.get('revision', release.name),
        # This override may already be present in a customized EnvironmentFile.
        **({'ORION_PROJECT_ROOT': str(release)} if 'ORION_PROJECT_ROOT' in read_env(text) else {}),
    })
    files[home / '.local/share/orion/studio-service/installed'] = str(release) + '\n'
    return files
