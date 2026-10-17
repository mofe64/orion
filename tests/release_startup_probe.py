"""Run real Python service startup with only capture/inference/server loops replaced."""
import importlib.util
import json
from pathlib import Path
import sys
from unittest.mock import MagicMock, patch

service, root, command_file = sys.argv[1:]
root = Path(root).resolve()
command = json.loads(Path(command_file).read_text())


def inside(path):
    path = Path(path).resolve()
    assert path.is_relative_to(root) and path.exists(), path
    return path


if service == 'orion-studio-gateway':
    program = Path(command[1])
    spec = importlib.util.spec_from_file_location('installed_gateway', program)
    gateway = importlib.util.module_from_spec(spec); spec.loader.exec_module(gateway)
    original = gateway.OrionGateway
    def construct(*args, **kwargs):
        instance = original(*args, **kwargs)
        assert instance._trajectory_compiler_path().resolve() == root / 'runtime/target/release/orion-trajectory'
        # Exercise the actual compiler with catalog data and release calibration.
        result = instance.compile_trajectory_preview({'motion': 'idle_breathe', 'start_pose': 'home'})
        assert result['format_version'] == 2 and result['samples']
        return instance
    with patch.object(sys, 'argv', command[1:]), patch.object(gateway, 'OrionGateway', side_effect=construct), \
         patch.object(gateway, 'GatewayHTTPServer') as server:
        gateway.main()
    server.return_value.serve_forever.assert_called_once()
elif service == 'orion-listener':
    from orion_voice import satellite, verifier, vad
    inside(satellite.__file__)
    import websockets.asyncio.server
    class Ready(Exception): pass
    class StopBeforeCapture:
        async def __aenter__(self): raise Ready()
        async def __aexit__(self, *args): pass
    def inference(path):
        inside(path)
        session = MagicMock(); session.get_inputs.return_value[0].name = 'input_1'
        return session
    def vad_model(path):
        assert Path(path).is_file() and Path(path).is_relative_to(root.parent.parent)
        return MagicMock()
    # File existence, verifier hashes, token checks, argument parsing and session
    # construction run normally. No camera, microphone or inference is opened.
    with patch.object(sys, 'argv', command), patch.object(verifier, 'onnx_session', side_effect=inference), \
         patch.object(vad, 'SileroModel', side_effect=vad_model), \
         patch.object(websockets.asyncio.server, 'serve', return_value=StopBeforeCapture()):
        try: satellite.main()
        except Ready: pass
        else: raise AssertionError('Listener did not reach its capture boundary')
elif service == 'orion-vision':
    from orion_vision import service as vision, detect_faces, detect_hands, detect_objects
    inside(vision.__file__)
    class Camera:
        def __init__(self, *args): pass
        def close(self): pass
    def detector(path):
        inside(path)
        return MagicMock()
    with patch.object(sys, 'argv', command), patch.object(vision, 'Picamera2Source', Camera), \
         patch.object(detect_faces, 'YuNet', side_effect=detector) as faces, \
         patch.object(detect_hands, 'PalmDetector', side_effect=detector) as palms, \
         patch.object(detect_objects, 'YoloNCNN', side_effect=detector) as objects, \
         patch.object(vision.Vision, 'start'), patch.object(vision.Vision, 'close'), \
         patch.object(vision, 'VisionServer') as server:
        vision.main()
    for model in (faces, palms, objects): model.assert_called_once()
    server.return_value.serve_forever.assert_called_once()
else:
    raise AssertionError(service)
print(service + ': startup ready')
