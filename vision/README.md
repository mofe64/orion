# Orion vision

## Setup on Ubuntu 24.04 / Raspberry Pi 5

The service owns the V2 IMX708 Wide camera and keeps frames in memory. It
serves HTTP on `/tmp/orion-vision.sock`; it performs no detection or movement.

Run from the Orion checkout on the Pi, with camera applications stopped:

```bash
sudo bash vision/scripts/install-camera-stack.sh "$USER"
```

The installer removes installed Ubuntu `libcamera*`, `python3-libcamera` and
`rpicam-apps*` packages, including packages apt must remove with them. It does
not run autoremove. It installs build prerequisites through apt, then builds
these Raspberry Pi releases into `/usr/local`:

| Component | Tag | Commit |
| --- | --- | --- |
| libpisp | `v1.2.0` | `50426319aa1a9ba4672f91977429365ad4e335a2` |
| libcamera | `v0.5.0+rpt20250429` | `d83ff0a4ae4503bc56b7ed48cd142c3dd423ad3b` |
| rpicam-apps | `v1.7.0` | `5a3f5965aca96c2d575261e4e4045f0e0481279b` |
| kms++ / pykms | pinned commit | `b91affd0725f2523e2563da4f6295a2977fd3945` |

The libcamera build enables only the `rpi/pisp` pipeline and IPA (image
processing algorithm), with Python bindings. The app build disables display
backends, video encoders and optional inference integrations. Re-running the
installer rebuilds the same releases, verifies their commit IDs, and replaces
the same Python path and udev rule files. Build sources are temporary.

The Python bindings live under `/usr/local/lib/python3.12/site-packages`.
`orion-libcamera.pth` in Ubuntu's local `dist-packages` makes them importable
with system Python. The DMA buffer rule gives group `video` access to
`/dev/dma_heap/*`; the optional installer argument joins that user to `video`.
Start a fresh login after the group change.

If the sensor is already detected with automatic camera detection, retain the
working boot configuration. Otherwise, set the camera overlay in
`/boot/firmware/config.txt` for the fitted port.
Use one explicit overlay, in the applicable `[all]` section, and disable
automatic camera detection when using it:

```ini
[all]
camera_auto_detect=0
# CAM1 (CAM/DISP1):
dtoverlay=imx708
# For CAM0 (CAM/DISP0), use dtoverlay=imx708,cam0 instead.
```

Reboot, then verify as the user who will own the camera:

```bash
bash vision/scripts/verify-camera-stack.sh
```

The check requires `imx708_wide` in `rpicam-hello --list-cameras`, an
importable `libcamera` module, and `rp1_cfe`, `pisp_be` and `imx708` in kernel
sysfs. Re-run it after kernel updates. A missing sensor usually needs a cable,
port or overlay check; a DMA allocation error needs a permissions or CMA
(contiguous memory allocator) check. Do not change the CMA reservation without
first recording the allocation failure.

## Service setup and development

Provision the native stack once with camera applications stopped, then create
an environment that sees Ubuntu's source-built `libcamera` and `pykms`:

```bash
uv venv --python /usr/bin/python3 --system-site-packages vision/.venv
uv sync --project vision --locked --python /usr/bin/python3
uv run --project vision orion-vision --profile hardware/profiles/v2.json
```

The Pi release builder performs the environment steps and installs the unit
only for profiles with `camera.enabled: true`. Provision native libraries
before deploying; release preparation does not replace shared libraries beneath
running services. The V2 profile selects camera index 0 on CAM/DISP1 with both
flips false. Retain the working `camera_auto_detect=1` boot configuration.

The service runs as the Pi user with supplementary group `video`, `Nice=10`,
`CPUQuota=60%`, and one OpenMP/OpenBLAS thread. The Unix socket is user-only
(mode 0600). Stop other camera applications before starting it.

| GET endpoint | Response |
| --- | --- |
| `/health` | Camera state, latest `capture_boottime_ns`, drops and connected streams |
| `/frame.jpg` | Latest main frame resized to 640×360; timestamp in `X-Capture-Boottime-Ns` |
| `/stream` | `multipart/x-mixed-replace`, boundary `orion-frame`, JPEG parts with length and timestamp |
| `/observations` | Empty list; detection is not implemented |

JPEGs are encoded on demand and cached across viewers at at most 15 fps.
Disconnected viewers cause encoding to stop. Capture converts RGB only while
a stream client is connected or a snapshot was requested in the last two
seconds. Idle callbacks publish metadata only, so health timestamps continue
without full-frame copies. The first viewing request waits up to one second
for an RGB frame. Capture retains a single latest RGB buffer, so slow consumers
drop frames rather than accumulating a queue.
`Picamera2Source` uses 1280×720 `BGR888`, whose Python byte order is RGB.
`make_array("main")` supplies the one owned copy when RGB is requested;
callbacks do not encode or write frames.

`capture_boottime_ns` names Linux `CLOCK_BOOTTIME`, which includes suspend.
It is `SensorTimestamp` (exposure start) plus `ExposureTime * 1000 / 2`
(microseconds to nanoseconds), rounded down. Missing/invalid metadata and
non-increasing sensor or midpoint timestamps drop the frame. Host receipt time
is never substituted. Matching these stamps to measured robot poses requires
the runtime pose-history interface.

The service watches `~/.config/orion/camera.json` every 100 ms. A missing file
defaults to enabled; malformed or unreadable settings fail closed. Setting
`camera_enabled` to false closes capture, purges buffers, ends existing streams
and makes every GET endpoint return HTTP 503 with `error.code: camera_off`.
Changing it to true reopens capture. No frames are saved. The setting is not
replaced by deployment or rollback. See [configuration](../docs/configuration.md).

Replay tests run on a workstation without Picamera2 or native Pi bindings:

```bash
uv run --project vision python -m unittest discover -s vision/tests -v
# For a synthetic JPEG fixture directory:
uv run --project vision orion-vision --profile hardware/profiles/v2.json \
  --replay /path/to/synthetic-fixtures --socket /tmp/orion-vision-test.sock
```

`FrameSource.start(callback, wants_rgb)` supplies capture metadata and an RGB
array only when the demand callback returns true; otherwise `Capture.rgb` is
`None`. `close()` stops capture. `ReplaySource` reads only synthetic JPEG fixtures.
`MujocoSource` raises an explicit not-implemented error. Picamera2 is restricted
to Linux aarch64 in the manifest. uv's exact metadata entry for the Linux-only
`python-prctl` sdist allows locking on a workstation; the Pi still builds it.

### Measurement environment

Picamera2 0.3.25 imports `pykms` even with preview disabled. The native
installer builds that binding and its required utils library; no display is
opened.

Use a disposable benchmark environment with the system bindings visible.
These packages belong to commissioning tooling; they do not establish a
vision-service dependency lockfile:

```bash
sudo apt-get install -y --no-install-recommends libcap-dev
uv venv --python /usr/bin/python3 --system-site-packages \
  "$HOME/.cache/orion/camera-benchmark"
uv pip install --python "$HOME/.cache/orion/camera-benchmark/bin/python" \
  picamera2==0.3.25 numpy==2.2.4 \
  opencv-python-headless==4.11.0.86 ncnn==1.0.20250503
# NCNN pulls in the desktop wheel, which shares cv2 with the headless one.
uv pip uninstall --python "$HOME/.cache/orion/camera-benchmark/bin/python" opencv-python
uv pip install --python "$HOME/.cache/orion/camera-benchmark/bin/python" \
  --reinstall-package opencv-python-headless opencv-python-headless==4.11.0.86
```

Picamera2 is installed without its desktop extras. NCNN's metadata declares
`opencv-python`; replacing that wheel with `opencv-python-headless` supplies
the same `cv2` API without requiring `libGL`. A dependency check will report
the intentionally absent desktop distribution. Avoid reinstalling NCNN
in this environment without repeating the replacement.

Provide a local OpenCV
YuNet ONNX model and a local **YOLO11n NCNN export made with `imgsz=320`**.
The export must have `in0` and `out0` blobs, or supply their names with
`--yolo-input` and `--yolo-output`. Models must already be on the Pi; the
benchmark never downloads or exports them. YOLO11n uses AGPL-3.0; product
integration remains a separate licence decision. The report records the
SHA-256 of every model input so runs with different assets are distinguishable.

```bash
"$HOME/.cache/orion/camera-benchmark/bin/python" vision/scripts/measure.py \
  --yunet /path/to/face_detection_yunet_2023mar.onnx \
  --yolo-param /path/to/yolo11n_ncnn_model/model.ncnn.param \
  --yolo-bin /path/to/yolo11n_ncnn_model/model.ncnn.bin \
  --duration 60 --label idle
```

Repeat with `--label voice` during a voice conversation, using the same
models, duration and cooling. `--camera` selects the enumerated index;
`--hflip` and `--vflip` record and apply a fitted image transform.

The camera produces main 1280×720 RGB and lores 320×240 YUV streams at a
requested 30 fps. Five equal timed windows measure capture alone, YuNet at
one and two threads, then YOLO at one and two threads. Startup, model loading,
one inference warmup per configuration, and shutdown are outside `--duration`.
A window can overrun by its last in-flight inference; actual elapsed time is
reported. Both stream counts come from their shared completed requests, and
sensor frame rate is reported separately from host delivery rate.

Inference consumes only the latest lores buffer. When it is slow, older
frames are replaced rather than queued. Timing includes YUV conversion and
removal of ISP stride padding after conversion to BGR, then
model execution; YuNet includes its detection postprocessing, while YOLO
includes 320×320 letterboxing and its forward pass but excludes NMS
(non-maximum suppression). Compare each model across thread counts rather
than treating these as identical end-to-end detector timings.

Stdout contains one JSON summary; library diagnostics use stderr. Captured
frames stay in memory, are never saved, and are never transmitted. CPU
temperature is sampled once a second; unavailable readings are `null`.
Failures emit `ok: false` and return a nonzero exit status.

`SensorTimestamp` is used directly, with no host-time fallback. The pinned
[libcamera control contract](https://github.com/raspberrypi/libcamera/blob/v0.5.0%2Brpt20250429/src/libcamera/control_ids_core.yaml)
uses `CLOCK_BOOTTIME`, which includes suspend time; it must not be assumed
equal to `CLOCK_MONOTONIC`. Reports include both host clocks and the
sensor-to-delivery interval. Exact exposure timing and clock alignment still
need lamp validation before camera observations are matched to robot poses.

### Lamp acceptance record

Record the fitted CSI port, overlay, kernel, model hashes, cooling and image
transform beside the idle and voice JSON summaries. With `rpicam-hello`
running, sweep base yaw and neck swivel across their **existing calibrated**
ranges and record any capture dropouts:

```bash
/usr/local/bin/rpicam-hello --nopreview --timeout 0
```

Check image orientation at `home` with a local in-memory viewer on the Pi.
The headless `rpicam-hello` build checks capture but cannot display orientation.
Do not infer the fitted transform from CAD or save frames for this check.

Measure Qwen ASR latency on comparable utterances with the benchmark off and
running. `--label voice` identifies the workload; it does not instrument Qwen.
The JSON's `qwen_asr_slowdown` remains `null` until that separate comparison
is recorded. Include capture rate, inference median/p95, ASR slowdown,
temperature, cable continuity and orientation in the commissioning results.
These hardware results are required before selecting detector frame rates.
