"""Plan a Pi release switch while preserving installed service settings."""
from pathlib import Path
import re
import shlex

from pi_hardware import profile, release_hardware

SERVICES = ('oriond', 'orion-studio-gateway', 'orion-listener', 'orion-voice-stack')
STOP_ORDER = tuple(reversed(SERVICES))
MANAGED_SERVICES = (*SERVICES, 'orion-vision', 'orion-spotify')
MANAGED_STOP_ORDER = tuple(reversed(MANAGED_SERVICES))
# Discard obsolete values when updating an existing installation.
REMOVED_VOICE_ENV = frozenset((
    'HF_HOME', 'HF_HUB_OFFLINE', 'HF_HUB_CACHE', 'HUGGINGFACE_HUB_CACHE',
    'ORION_STUDIO_TTS_MODEL',
))


def services_for_release(release):
    camera = profile(release_hardware(release), project=release).get('camera', {})
    return (*SERVICES, *(('orion-vision',) if camera.get('enabled') is True else ()),
            *(('orion-spotify',) if (release / 'scripts/spotify_connect.py').is_file() else ()))

REFERENCE_WAKE_MODEL = 'hey_orion_reference.rpw'
# The reference model proposes candidates. The ASR wake check confirms them;
# the packaged acoustic verifier (voice/models/verifier) stays off by default.
ACTIVE_WAKE_MODEL = REFERENCE_WAKE_MODEL
ACTIVE_WAKE_THRESHOLD = '0.35'
# Recognize the legacy managed threshold so upgrades keep following the active profile.
MANAGED_WAKE_THRESHOLDS = frozenset((float(ACTIVE_WAKE_THRESHOLD), 0.8))
# Asset-pack paths stay in the catalog; only app resources move to the release.
PATH_SUFFIXES = (
    'runtime/target/release/oriond', 'runtime/target/release/orion-trajectory',
    'orion-service/target/release/orion-service', 'studio-service/target/release/orion-studio-headless',
    'voice/.venv/bin/orion-listener', 'orion_studio/gateway.py',
    'vision/.venv/bin/orion-vision', 'orion_studio/.venv/bin/python',
    'voice/models/wake/hey_orion_trained_080.rpw',
    'voice/models/wake/hey_orion_reference.rpw',
    'voice/models/verifier',
    'scripts/wait_for_oriond.sh', 'scripts/orion_safe_stop.sh',
    'scripts/spotify_connect.py', 'native/librespot',
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


def merge_env(text, defaults, removed=()):
    """Drop retired keys and add missing defaults; preserve tuning and comments.

    A removed key that also has a default is replaced by that default, so a
    release-specific path moves to the new release instead of disappearing.
    """
    existing = {key: value for key, value in read_env(text).items() if key not in removed}
    lines = []
    for line in text.splitlines(keepends=True):
        key = line.strip().partition('=')[0]
        if key not in removed:
            lines.append(line)
    result = ''.join(lines)
    if result and not result.endswith('\n'):
        result += '\n'
    for key, value in defaults.items():
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


def listener_start(start, release):
    """Update only known managed wake profiles; keep independent operator tuning."""
    model_flag = re.search(r'(?<!\S)--wake-model(?:\s+|=)(\S+)', start)
    threshold_flag = re.search(r'(?<!\S)--threshold(?:\s+|=)(\S+)', start)
    model = Path(model_flag[1]).name if model_flag else None
    try:
        threshold = float(threshold_flag[1]) if threshold_flag else None
    except ValueError:
        threshold = None
    managed = ((model is None and (threshold in MANAGED_WAKE_THRESHOLDS or threshold_flag is None)) or
               (model == REFERENCE_WAKE_MODEL and threshold in MANAGED_WAKE_THRESHOLDS))
    if managed:
        wanted_model = f'{release}/voice/models/wake/{ACTIVE_WAKE_MODEL}'
        if threshold_flag:
            start = start[:threshold_flag.start()] + f'--threshold {ACTIVE_WAKE_THRESHOLD}' + start[threshold_flag.end():]
        else:
            start += f' --threshold {ACTIVE_WAKE_THRESHOLD}'
        model_flag = re.search(r'(?<!\S)--wake-model(?:\s+|=)(\S+)', start)
        if model_flag:
            start = start[:model_flag.start()] + f'--wake-model {wanted_model}' + start[model_flag.end():]
        else:
            start += f' --wake-model {wanted_model}'
    elif model is None:
        start += f' --wake-model {release}/voice/models/wake/{REFERENCE_WAKE_MODEL}'
    # Wheel-installed code lives in site-packages; models stay in the release.
    # The packaged verifier is replaced by the ASR wake check. A custom
    # verifier directory is an operator choice and stays.
    start = re.sub(r'(?<!\S)--verifier-dir(?:\s+|=)\S*voice/models/verifier/?(?=\s|$)', '--no-verifier', start)
    if not re.search(r'(?<!\S)--(?:verifier-dir(?:\s|=)|no-verifier(?:\s|$))', start):
        start += ' --no-verifier'
    if '--local-processor' not in start:
        start += ' --local-processor'
    return start



def set_option(start, flag, value):
    """Replace a managed single-value argument without losing operator tuning."""
    pattern = r'(?<!\S)' + re.escape(flag) + r'(?:\s+|=)(?:"[^"]*"|\'[^\']*\'|\S+)'
    updated, count = re.subn(pattern, lambda _: f'{flag} {value}', start)
    return updated if count else start + f' {flag} {value}'


def change_effective_start(contents, transform):
    start = effective_start(contents)
    for i in range(len(contents) - 1, -1, -1):
        p, value = contents[i]
        if re.search(r'^ExecStart=.+', value, re.M):
            contents[i] = (p, re.sub(r'^ExecStart=.+$',
                lambda match: 'ExecStart=' + transform(start) if match.group()[10:] == start else match.group(), value, flags=re.M))
            return
    raise ValueError('Service has no effective ExecStart')

def render_plan(release, root, runtime_project, home, user, unit_dir=Path('/etc/systemd/system'), system_root=Path('/')):
    for path in [release, root, runtime_project, home]:
        if not re.fullmatch(r'/[A-Za-z0-9._/-]+', str(path)) or '..' in path.parts:
            raise ValueError(f'Unsupported service path: {path}')
    if not re.fullmatch(r'[A-Za-z_][A-Za-z_0-9-]*', user):
        raise ValueError('Unsupported service user')
    hardware = release_hardware(release)
    selected = profile(hardware)
    calibration = release / selected["joint_profile"] if hardware == "v2" else home / ".config/orion" / selected["calibration_file"]
    files = {}
    for name in services_for_release(release):
        path = unit_dir / f'{name}.service'
        if path.is_symlink():
            raise ValueError(f'Inspect symlinked service before deployment: {path}')
        existing = path.exists()
        text = path.read_text() if existing else (release / f'scripts/systemd/{name}.service.in').read_text()
        for key, value in {'PROJECT_ROOT': runtime_project, 'USER_HOME': home, 'ORION_USER': user}.items():
            text = text.replace('@' + key + '@', str(value))
        if name == 'oriond' and 'Requires=orion-neopixel-pin.service' not in text:
            text = text.replace('[Unit]', '[Unit]\nRequires=orion-neopixel-pin.service', 1)
        overrides = sorted((unit_dir / f'{name}.service.d').glob('*.conf'))
        contents = [(path, text)]
        for override in overrides:
            if override.is_symlink():
                raise ValueError(f'Inspect symlinked override before deployment: {override}')
            contents.append((override, override.read_text()))
        combined = '\n'.join(value for _, value in contents)
        if name in ('oriond', 'orion-studio-gateway', 'orion-listener'):
            def hardware_start(start):
                start = set_option(start, '--hardware', hardware)
                if name in ('oriond', 'orion-studio-gateway'):
                    start = set_option(start, '--calibration', calibration)
                if name == 'orion-studio-gateway':
                    start = set_option(start, '--project-root', runtime_project)
                if name == 'oriond':
                    start = start.replace(' --port /dev/ttyACM0', '')
                    # The runtime now always uses ~/.config/orion/routines.json.
                    start = re.sub(r'(?<!\S)--routines-file(?:\s+|=)(?:"[^"]*"|\'[^\']*\'|\S+)', '', start)
                    start = set_option(start, '--poses', runtime_project / selected['poses'])
                    for flag, field in (('--motions', 'motions'), ('--scenes', 'scenes'), ('--user-poses', 'user_poses')):
                        start = set_option(start, flag, runtime_project / selected[field])
                    start = set_option(start, '--cues', runtime_project / 'audio/cues')
                    start = set_option(start, '--audio-card', selected['audio']['card'])
                    start = set_option(start, '--audio-device', selected['audio']['pcm'])
                    if hardware == 'v2':
                        start = set_option(start, '--camera-urdf', release / 'description/urdf/orion-v2.urdf')
                        # Commissioned V2 boots into character mode like V1. Earlier
                        # releases wrote 'off' into the unit, so replace it explicitly.
                        start = set_option(start, '--character-on-start', 'on')
                if name == 'orion-listener':
                    start = set_option(start, '--device', selected['audio']['pcm'])
                    if hardware == 'v2':
                        audio = selected['audio']
                        if '--capture-channels' not in start:
                            start = set_option(start, '--capture-channels', audio['capture_channels'])
                        if '--processed-channel' not in start:
                            start = set_option(start, '--processed-channel', audio['processed_channel'])
                    else:
                        start = set_option(start, '--capture-channels', 2)
                        start = set_option(start, '--processed-channel', 0)
                return start
            change_effective_start(contents, hardware_start)
        if name == 'orion-vision':
            change_effective_start(contents, lambda start: set_option(start, '--profile',
                release / 'hardware/profiles' / f'{hardware}.json'))
            change_effective_start(contents, lambda start: set_option(start, '--face-model',
                release / 'vision/models/face_detection_yunet_2023mar.onnx'))
            if (release / 'vision/orion_vision/detect_hands.py').is_file():
                change_effective_start(contents, lambda start: set_option(start, '--palm-model',
                    release / 'vision/models/palm_detection_mediapipe_2023feb.onnx'))
            if (release / 'vision/orion_vision/detect_objects.py').is_file():
                change_effective_start(contents, lambda start: set_option(start, '--object-model',
                    release / 'vision/models/yolo11n_ncnn_model'))

        # A first onboard installation adds only missing defaults. Existing override values win.
        if name == 'orion-listener':
            start = effective_start(contents)
            desired = listener_start(start, release)
            if desired != start:
                # Change only the effective ExecStart; preserve the other overrides.
                for i in range(len(contents) - 1, -1, -1):
                    p, value = contents[i]
                    if re.search(r'^ExecStart=.+', value, re.M):
                        contents[i] = (p, re.sub(r'^ExecStart=.+$',
                            lambda match: 'ExecStart=' + desired if match.group()[10:] == start else match.group(),
                            value, flags=re.M))
                        break
            additions = []
            if 'ORION_VAD_MODEL=' not in combined:
                additions.append(f'Environment=ORION_VAD_MODEL={root}/models/silero.onnx')
            if 'ORION_CAPTURE_GAIN_DB=' not in combined:
                additions.append('Environment=ORION_CAPTURE_GAIN_DB=25')
            if additions:
                p, value = contents[0]
                contents[0] = (p, value.replace('[Service]', '[Service]\n' + '\n'.join(additions), 1))
        check_command = [str(release / 'runtime/target/release/oriond'), '--check', '--hardware', hardware, '--calibration', str(calibration)]
        if name == 'oriond':
            start_arguments = shlex.split(effective_start(contents))
            for flag in ('--port', '--baud-rate'):
                for i, argument in enumerate(start_arguments):
                    if argument == flag and i + 1 < len(start_arguments):
                        check_command.extend((flag, start_arguments[i + 1]))
                    elif argument.startswith(flag + '='):
                        check_command.extend((flag, argument.partition('=')[2]))
        for p, value in contents:
            value = rebase_commands(value, release)
            if name == 'orion-studio-gateway':
                value = value.replace('/usr/bin/python3 ', str(release / 'orion_studio/.venv/bin/python') + ' ')
            if name == 'oriond' and p == path:
                value = re.sub(r'^ExecStartPre=.*oriond --check.*\n?', '', value, flags=re.M)
                value = value.replace('[Service]', '[Service]\nExecStartPre=' + shlex.join(check_command), 1)

            if name in ('orion-listener', 'orion-voice-stack'):
                directory = release / 'voice' if name == 'orion-listener' else release
                value = re.sub(r'^WorkingDirectory=.*$', f'WorkingDirectory={directory}', value, flags=re.M)
            if name == 'orion-vision':
                value = re.sub(r'^WorkingDirectory=.*$', f'WorkingDirectory={release}', value, flags=re.M)
            files[p] = value
    environment_path = home / '.config/orion/voice-stack.env'
    text = environment_path.read_text() if environment_path.exists() else ''
    defaults = {
        'ORION_STUDIO_CODEX_BIN': str(root / 'codex-0.157.0/bin/codex'),
        'ORION_ASR_MODEL_DIR': str(root / 'models/qwen'), 'ORION_LLAMA_SERVER': str(release / 'native/llama-server'),
        'ORION_ASR_THREADS': '3', 'ORION_TTS_THREADS': '3',
        'ORION_PIPER_MODEL_DIR': str(root / 'models/piper-alba-medium'),
    }
    files[environment_path] = merge_env(text, defaults, REMOVED_VOICE_ENV | {'ORION_LLAMA_SERVER'})
    helper = system_root / 'usr/local/libexec/orion-home-assistant.py'
    manifest = system_root / 'usr/local/share/orion/home-assistant.json'
    files[helper] = (release / 'scripts/home_assistant_install.py').read_text()
    files[manifest] = (release / 'scripts/release-home-assistant.json').read_text()
    files[unit_dir / 'orion-home-assistant-install.service'] = (
        '[Unit]\nDescription=Install independent Orion Home Assistant container\nAfter=network-online.target\nWants=network-online.target\n'
        '[Service]\nType=oneshot\nTimeoutStartSec=900\nUMask=0077\n'
        f'ExecStart=/usr/bin/python3 {helper} --user {user} --manifest {manifest}\n')
    files[system_root / 'etc/polkit-1/rules.d/60-orion-home-assistant.rules'] = (
        'polkit.addRule(function(action, subject) {\n'
        '  if (action.id === "org.freedesktop.systemd1.manage-units" &&\n'
        '      action.lookup("unit") === "orion-home-assistant-install.service" &&\n'
        f'      action.lookup("verb") === "start" && subject.user === "{user}") {{\n'
        '    return polkit.Result.YES;\n  }\n});\n')
    files[home / '.config/orion/hardware'] = hardware + '\n'
    files[home / '.local/share/orion/studio-service/installed'] = str(release) + '\n'
    return files
