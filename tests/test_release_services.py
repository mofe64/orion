"""All managed services start from separate app and asset roots, without checkout files."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import shlex
import socket
from string import Template
import subprocess
import sys
import time
import tempfile
import tomllib
import venv

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
from pi_service_config import MANAGED_SERVICES, effective_start, read_env, render_plan
from release_assets import ASSET_DIRECTORIES, remove_assets
from pi_catalog import built_in_asset


def binary(component, name):
    target = Path(os.environ.get('CARGO_TARGET_DIR', ROOT / component / 'target'))
    for mode in (('release', 'debug') if os.environ.get('CARGO_TARGET_DIR') else ('debug', 'release')):
        path = target / mode / name
        if path.is_file(): return path
    raise AssertionError(f'Build {component} before the release layout test: missing {name}')


def environment(release, component):
    """Install Orion code in the fixture; reuse only third-party test dependencies."""
    destination = release / component / '.venv'
    venv.EnvBuilder(with_pip=False).create(destination)
    purelib = Path(subprocess.check_output([str(destination / 'bin/python'), '-c',
        'import sysconfig; print(sysconfig.get_path("purelib"))'], text=True).strip())
    package = {'voice': 'orion_voice', 'vision': 'orion_vision', 'speech': 'orion_speech_worker'}.get(component)
    if package:
        shutil.copytree(ROOT / component / package, purelib / package, ignore=shutil.ignore_patterns('__pycache__'))
    if component in ('voice', 'vision'):
        # release_build prepares these wheel environments before running this test.
        dependencies = next((p for p in (Path('/tmp') / ('test-' + component) / 'bin/python',
                                         ROOT / component / '.venv/bin/python') if p.is_file()), None)
        assert dependencies, f'Prepare the {component} wheel test environment first'
        packages = subprocess.check_output([str(dependencies), '-c',
            'import sysconfig; print(sysconfig.get_path("purelib"))'], text=True).strip()
        (purelib / 'third_party_test_dependencies.pth').write_text(packages + '\n')
    return destination / 'bin/python'


def run(command, cwd, env):
    result = subprocess.run(list(map(str, command)), cwd=cwd, env=env, capture_output=True, text=True, timeout=30)
    assert result.returncode == 0, f'{command}\n{result.stdout}\n{result.stderr}'
    return result


def test_every_rendered_service_starts_from_release_and_catalog_without_a_checkout():
    # Keep Unix sockets under the platform's sockaddr_un path limit.
    with tempfile.TemporaryDirectory(prefix='orion-layout-', dir='/tmp') as folder:
        base = Path(folder).resolve(); home = base / 'home'; home.mkdir()
        root = home / '.local/share/orion/voice-stack'; release = root / 'releases/test'
        catalog = root / 'catalog'; release.mkdir(parents=True); catalog.mkdir()
        ignore = shutil.ignore_patterns('__pycache__', '.venv', 'target', 'node_modules', 'models')
        for directory in ('hardware/profiles', 'description/urdf', 'scripts/systemd',
                          'voice/orion_voice', 'vision/orion_vision'):
            shutil.copytree(ROOT / directory, release / directory, ignore=ignore)
        studio_modules = tomllib.loads((ROOT / 'orion_studio/pyproject.toml').read_text())['tool']['setuptools']['py-modules']
        for relative in (*(f'orion_studio/{module}.py' for module in studio_modules), 'runtime/mujoco_bridge.py',
                         'scripts/home_assistant_install.py', 'scripts/release-home-assistant.json', 'scripts/spotify_connect.py'):
            path = release / relative; path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / relative, path)
        (release / 'release.json').write_text(json.dumps({'revision': 'layout-test', 'hardware': 'v2',
            'version': (ROOT / 'VERSION').read_text().strip()}))
        # Build-style removal leaves no built-ins beside the executable.
        for directory in ASSET_DIRECTORIES:
            shutil.copytree(ROOT / directory, release / directory, ignore=shutil.ignore_patterns('user', '__pycache__'))
            (catalog / directory).mkdir(parents=True)
            for path in (release / directory).rglob('*'):
                relative = path.relative_to(release)
                if path.is_file() and built_in_asset(relative):
                    target = catalog / relative; target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(path, target)
        remove_assets(release)
        for directory in ASSET_DIRECTORIES:
            assert not any(p.is_file() and built_in_asset(p.relative_to(release)) for p in (release / directory).rglob('*'))
        for tree in (release, catalog):
            assert not list(tree.rglob('AGENTS.md')) and not list(tree.rglob('.git'))
        assert not (catalog / 'runtime').exists()
        for component, names in {'runtime': ('oriond', 'orion-trajectory'), 'orion-service': ('orion-service',)}.items():
            for name in names:
                target = release / component / 'target/release' / name
                target.parent.mkdir(parents=True, exist_ok=True); shutil.copy2(binary(component, name), target)
        interpreters = {c: environment(release, c) for c in ('speech', 'voice', 'vision', 'orion_studio')}
        # Replace the network receiver, while executing the real launcher and
        # rendered service command from the isolated release.
        receiver = release / 'native/librespot'; receiver.parent.mkdir()
        receiver.write_text(f'#!{interpreters["orion_studio"]}\n' +
            'import sys,time\n' +
            'assert sys.argv[sys.argv.index("--backend")+1]=="pipe"\n' +
            'assert sys.argv[sys.argv.index("--format")+1]=="S16"\n' +
            'assert sys.argv[sys.argv.index("--volume-ctrl")+1]=="fixed"\n' +
            'sys.stdout.buffer.write((2000).to_bytes(2,"little",signed=True)*882)\n' +
            'sys.stdout.buffer.flush()\ntime.sleep(30)\n')
        receiver.chmod(0o755)
        receiver.with_suffix('.sha256').write_text(hashlib.sha256(receiver.read_bytes()).hexdigest())
        for component, executable, module in (('voice', 'orion-listener', 'orion_voice.satellite'),
                                              ('vision', 'orion-vision', 'orion_vision.service')):
            script = release / component / '.venv/bin' / executable
            script.write_text(f'#!{interpreters[component]}\nfrom {module} import main\nmain()\n'); script.chmod(0o755)
        # The wake detector reads its real model; inference sessions below are boundary doubles.
        wake = release / 'voice/models/wake'; wake.mkdir(parents=True)
        shutil.copy2(ROOT / 'voice/models/wake/hey_orion_reference.rpw', wake)
        verifier = release / 'voice/models/verifier'; verifier.mkdir()
        config = json.loads((ROOT / 'voice/models/verifier/config.json').read_text())
        for name in config['files']:
            model = verifier / name; model.write_bytes(b'startup model fixture')
            config['files'][name] = hashlib.sha256(model.read_bytes()).hexdigest()
        (verifier / 'config.json').write_text(json.dumps(config))
        for name in ('face_detection_yunet_2023mar.onnx', 'palm_detection_mediapipe_2023feb.onnx', 'yolo11n_ncnn_model'):
            model = release / 'vision/models' / name; model.parent.mkdir(parents=True, exist_ok=True)
            if name.endswith('.onnx'): model.write_bytes(b'startup model fixture')
            else: model.mkdir()
        settings = home / '.config/orion'; settings.mkdir(parents=True)
        (settings / 'studio-token').write_text('release-layout-pairing-token-' * 2)
        (settings / 'studio-token').chmod(0o600)
        units = base / 'units'; units.mkdir()
        files = render_plan(release, root, catalog, home, 'pi', units)
        shared = root / 'models/qwen'; shared.mkdir(parents=True)
        (root / 'models/silero.onnx').write_bytes(b'startup model fixture')
        env = {**os.environ, **read_env(files[settings / 'voice-stack.env']), 'HOME': str(home),
               'PYTHONDONTWRITEBYTECODE': '1', 'ORION_TEST_DRIVER_CONTROL': str(base / 'driver-control.json'),
               'ORION_TEST_DRIVER_EVENTS': str(base / 'driver-events.jsonl')}
        for key in ('PYTHONPATH', 'ORION_TRAJECTORY_COMPILER', 'ORION_PROJECT_ROOT'):
            env.pop(key, None)
        commands, environments = {}, {}
        for name in MANAGED_SERVICES:
            text = files[units / f'{name}.service']
            selected_env = env.copy()
            for line in text.splitlines():
                if line.startswith('Environment='):
                    for assignment in shlex.split(line.partition('=')[2]):
                        key, _, value = assignment.partition('='); selected_env[key] = value
            environments[name] = selected_env
            commands[name] = [Template(arg).substitute(selected_env) for arg in
                              shlex.split(effective_start([(units / f'{name}.service', text)]))]
        for command in commands.values():
            assert Path(command[0]).is_file(), command
        checked = set()
        # Native service startup performs executable-relative release discovery and
        # checks the staged speech venv in isolated Python mode.
        run([commands['orion-voice-stack'][0], 'check'], release, environments['orion-voice-stack']); checked.add('orion-voice-stack')
        # The daemon executes its actual serve path with only servo/audio/light IO substituted.
        driver = base / 'following_driver.py'; shutil.copy2(ROOT / 'runtime/tests/fixtures/following_driver.py', driver)
        driver.chmod(0o755)
        daemon_socket = base / 'runtime.sock'
        commands['orion-vision'] += ['--socket', str(base / 'vision.sock'), '--runtime-socket', str(daemon_socket)]
        daemon = commands['oriond'].copy(); daemon[daemon.index('--backend')+1] = 'mujoco'
        daemon += ['--python', str(driver), '--socket', str(daemon_socket), '--start-pose', 'home', '--character-on-start', 'off']
        with tempfile.TemporaryFile(mode='w+') as output:
            process = subprocess.Popen(daemon, cwd=catalog, env=environments['oriond'], stdout=output, stderr=output, text=True)
            try:
                deadline = time.monotonic() + 15
                while not daemon_socket.exists() and process.poll() is None and time.monotonic() < deadline:
                    time.sleep(0.02)
                if not daemon_socket.exists():
                    output.seek(0); raise AssertionError(output.read())
                with socket.socket(socket.AF_UNIX) as client:
                    client.settimeout(3); client.connect(str(daemon_socket)); client.sendall(b'status\n')
                    with client.makefile('rb') as response:
                        assert json.loads(response.readline(1024 * 1024))['mode'] in ('observe', 'holding', 'configured')
                checked.add('oriond')
                with socket.socket(socket.AF_UNIX) as music:
                    music_socket = base / 'music.sock'; music.bind(str(music_socket)); music.listen(1); music.settimeout(5)
                    spotify = subprocess.Popen([*commands['orion-spotify'], '--socket', str(music_socket)],
                        cwd=catalog, env=environments['orion-spotify'], stdout=output, stderr=output)
                    try:
                        with music.accept()[0] as stream:
                            stream.settimeout(5); pcm = b''
                            while len(pcm) < 1764:
                                chunk = stream.recv(1764-len(pcm))
                                assert chunk, 'Spotify launcher closed before forwarding PCM'
                                pcm += chunk
                        assert pcm == (2000).to_bytes(2,'little',signed=True)*882
                        assert spotify.poll() is None
                        checked.add('orion-spotify')
                    finally:
                        spotify.terminate()
                        try: spotify.wait(timeout=5)
                        except subprocess.TimeoutExpired: spotify.kill(); spotify.wait(timeout=5)
                for name, component in (('orion-studio-gateway', 'orion_studio'), ('orion-listener', 'voice'), ('orion-vision', 'vision')):
                    command_file = base / (name + '.json'); command_file.write_text(json.dumps(commands[name]))
                    result = run([interpreters[component], ROOT / 'tests/release_startup_probe.py', name, release, command_file],
                                 catalog if name == 'orion-studio-gateway' else release, environments[name])
                    assert name + ': startup ready' in result.stdout
                    checked.add(name)
            finally:
                process.terminate()
                try: process.wait(timeout=5)
                except subprocess.TimeoutExpired: process.kill(); process.wait(timeout=5)
        assert checked == set(MANAGED_SERVICES)
