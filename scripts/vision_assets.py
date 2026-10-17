"""Stage the commissioned export; no model conversion or downloads on the lamp."""
import ast
import hashlib
import os
from pathlib import Path
import shutil
import sys
import subprocess


def hailo_models():
    # The installer reads the same standard-library-only pins as the wheel.
    source = str(Path(__file__).resolve().parents[1] / 'vision')
    if source not in sys.path:
        sys.path.insert(0, source)
    from orion_vision import hailo_models as models
    return models


def prepare_hailo_models(root, present, run, directory=None):
    if not present:
        return
    models = hailo_models()
    directory = Path(directory) if directory is not None else models.MODEL_DIRECTORY
    try:
        architecture = models.device_architecture()
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f'Hailo identification unavailable; vision will use CPU: {error}')
        return
    if architecture is None:
        return
    from prepare_pi_voice_stack import download, sha256
    for entry in models.model_manifest()[architecture].values():
        target = directory / entry['path']
        if target.exists():
            if sha256(target) != entry['sha256']:
                raise RuntimeError(f'Hailo HEF SHA-256 mismatch: {target}')
            continue
        source = download(root, entry['url'], 'models/hailo/' + entry['path'], entry['sha256'])
        run('sudo', 'install', '-d', target.parent)
        run('sudo', 'install', '-m', '0644', source, target)


def yolo_hashes(release):
    # Read the selected commit's pins without importing cv2/NCNN on the builder.
    module = ast.parse((Path(release) / 'vision/orion_vision/detect_objects.py').read_text())
    return next(ast.literal_eval(node.value) for node in module.body if isinstance(node, ast.Assign)
                and any(isinstance(target, ast.Name) and target.id == 'MODEL_HASHES' for target in node.targets))


def verify_yolo(release, directory=None):
    directory = Path(directory) if directory is not None else Path(release) / 'vision/models/yolo11n_ncnn_model'
    for name, digest in yolo_hashes(release).items():
        path = directory / name
        if not path.is_file():
            raise RuntimeError(f'Missing pinned YOLO11n export: {path}. Stage the P0 export using ORION_YOLO_MODEL_DIR; see vision/README.md.')
        with path.open('rb') as source:
            if hashlib.file_digest(source, 'sha256').hexdigest() != digest:
                raise RuntimeError(f'YOLO11n SHA-256 mismatch: {path}')
    return directory


def stage_yolo(source, release):
    source = Path(os.environ.get('ORION_YOLO_MODEL_DIR', Path(source) / 'vision/models/yolo11n_ncnn_model')).expanduser()
    verify_yolo(release, source)
    destination = Path(release) / 'vision/models/yolo11n_ncnn_model'
    destination.mkdir(parents=True, exist_ok=True)
    for name in yolo_hashes(release):
        shutil.copy2(source / name, destination / name)
    verify_yolo(release)
