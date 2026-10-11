"""Sources deliver metadata and requested RGB; the service validates timestamps."""
from dataclasses import dataclass
from pathlib import Path
from threading import Event, Thread
from typing import Callable, Protocol
import time

import numpy as np
import simplejpeg

from .intrinsics import SIZE, camera_geometry, stream_geometry


@dataclass(frozen=True)
class Capture:
    rgb: np.ndarray | None
    metadata: dict
    lores: np.ndarray | None = None
    geometry: dict | None = None


class FrameSource(Protocol):
    def start(self, publish: Callable[[Capture], None], wants_rgb: Callable[[], bool],
              wants_lores: Callable[[], bool] | None = None) -> None: ...
    def close(self) -> None: ...


class Picamera2Source:
    def __init__(self, index=0, hflip=False, vflip=False):
        if hflip or vflip:
            raise ValueError("Camera flips are unsupported by fisheye rays; both must be false")
        self.index, self.hflip, self.vflip = index, hflip, vflip
        self.camera = None
        self.expected_geometry = camera_geometry()

    def start(self, publish, wants_rgb, wants_lores=None):
        from picamera2 import Picamera2
        from libcamera import Transform

        self.camera = Picamera2(self.index)
        try:
            # libcamera BGR888 is [R,G,B] in a Python array, despite its name.
            streams = {"lores": {"size": SIZE, "format": "YUV420", "preserve_ar": False}} if wants_lores else {}
            controls = {"FrameRate": 30, "ScalerCrop": tuple(self.expected_geometry['scaler_crop'])}
            config = self.camera.create_video_configuration(
                main={"size": (1280, 720), "format": "BGR888"},
                **streams, sensor=self.expected_geometry['sensor_mode'],
                transform=Transform(hflip=self.hflip, vflip=self.vflip),
                controls=controls, buffer_count=4, queue=False)
            self.camera.configure(config)
            geometry = None
            if wants_lores:
                actual = self.camera.camera_configuration()
                lores = actual['lores']
                if (tuple(lores['size']) != SIZE or lores['format'] != 'YUV420'
                        or lores.get('preserve_ar', False)):
                    raise ValueError(f"Configured lores geometry differs from {SIZE[0]}x{SIZE[1]} YUV420")
                geometry = stream_geometry({'ScalerCrop': self.expected_geometry['scaler_crop']}, {
                    'sensor_mode': actual['sensor'], 'lores_stride': lores['stride'],
                    'flips': {'hflip': self.hflip, 'vflip': self.vflip}})
                if geometry['sensor_mode'] != self.expected_geometry['sensor_mode']:
                    raise ValueError("Configured sensor geometry differs from the built-in camera configuration")
                # Share the configured stride with the pipeline before any
                # callbacks can submit frames. Padding changes storage, not rays.
                self.expected_geometry['lores_stride'] = geometry['lores_stride']
                del geometry['scaler_crop']

            def completed(request):
                metadata = request.get_metadata()
                # make_array already owns a copy of the camera buffer. Idle
                # capture publishes metadata without converting the main frame.
                rgb = request.make_array("main") if wants_rgb() else None
                lores = request.make_array("lores") if wants_lores and wants_lores() else None
                publish(Capture(rgb, metadata, lores, geometry))

            self.camera.post_callback = completed
            self.camera.start(show_preview=False)
        except BaseException:
            self.close()
            raise

    def close(self):
        camera, self.camera = self.camera, None
        if camera is not None:
            camera.post_callback = None
            try:
                camera.stop()
            finally:
                camera.close()


class ReplaySource:
    """Loop synthetic JPEG inputs. Do not use this to archive camera capture."""
    def __init__(self, directory: Path, fps=30):
        self.paths = sorted(Path(directory).glob("*.jpg"))
        if not self.paths:
            raise ValueError("Replay directory must contain synthetic .jpg test images")
        self.period = 1 / fps
        self.stop = Event()
        self.thread = None

    def start(self, publish, wants_rgb, wants_lores=None):
        self.stop.clear()
        geometry = camera_geometry()
        crop = geometry.pop('scaler_crop')

        def replay():
            index = 0
            while not self.stop.is_set():
                rgb_wanted, lores_wanted = wants_rgb(), bool(wants_lores and wants_lores())
                image = simplejpeg.decode_jpeg(self.paths[index % len(self.paths)].read_bytes()) if rgb_wanted or lores_wanted else None
                rgb = image if rgb_wanted else None
                lores = None
                if lores_wanted:
                    height, width = image.shape[:2]
                    lores = np.ascontiguousarray(image[np.arange(SIZE[1]) * height // SIZE[1]]
                        [:, np.arange(SIZE[0]) * width // SIZE[0]])
                # On Linux this is the same clock as SensorTimestamp; Replay is
                # test input on hosts without CLOCK_BOOTTIME as well.
                from .gaze_client import boottime_ns
                publish(Capture(rgb, {"SensorTimestamp": boottime_ns(), "ExposureTime": 0,
                    "ScalerCrop": crop}, lores, geometry))
                index += 1
                self.stop.wait(self.period)

        self.thread = Thread(target=replay, name="vision-replay", daemon=True)
        self.thread.start()

    def close(self):
        self.stop.set()
        if self.thread is not None:
            self.thread.join(timeout=.5)
            self.thread = None


class MujocoSource:
    def start(self, publish, wants_rgb):
        raise NotImplementedError("MuJoCo camera capture is not implemented")

    def close(self):
        pass
