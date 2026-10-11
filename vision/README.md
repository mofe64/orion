# Orion vision

## Setup on Ubuntu 24.04 / Raspberry Pi 5

The service owns the V2 IMX708 Wide camera and keeps frames in memory. It
serves HTTP on `/tmp/orion-vision.sock`. With fitted intrinsics and the pinned
YuNet model, it tracks faces and sends base bearings to runtime gaze.

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
| `/observations` | Recent base-bearing face tracks, normalized boxes, pixel landmarks and capture timestamps |
| `/speaker?since_ns=…` | Chosen face and selection strategy within a bounded mouth-motion window |

JPEGs are encoded on demand and cached across viewers at at most 15 fps.
Disconnected viewers cause encoding to stop. Capture converts RGB only while
a stream client is connected or a snapshot was requested in the last two
seconds. Callbacks without viewing demand omit the main RGB copy. Tracking
copies the 320×240 lores YUV buffer independently, so detection continues
without full-frame RGB copies. The first viewing request waits up to one second
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

`FrameSource.start(callback, wants_rgb, wants_lores=None)` supplies capture metadata and an RGB
array only when the demand callback returns true; otherwise `Capture.rgb` is
`None`. Optional lores demand supplies a copied packed YUV420 buffer for the
worker (RGB for synthetic replay). `close()` stops capture. `ReplaySource` reads only synthetic JPEG fixtures.
`MujocoSource` raises an explicit not-implemented error. Picamera2 is restricted
to Linux aarch64 in the manifest. uv's exact metadata entry for the Linux-only
`python-prctl` sdist allows locking on a workstation. The local release builder
compiles it into a wheel before transfer to the Pi.

## Face calibration and tracking

The native installer downloads YuNet 2023mar from OpenCV Zoo commit
`47534e27c9851bb1128ccc0102f1145e27f23f98` to
`/usr/local/share/orion/vision/face_detection_yunet_2023mar.onnx`, verifying
SHA-256 `8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4`
before replacement. The service verifies that hash again before model loading.
OpenCV headless is pinned to 4.11.0.86 in the vision lockfile.

Stop the vision service before calibration so the camera has one owner:

```bash
systemctl --user stop orion-vision.service
uv run --project vision orion-vision-calibrate --profile hardware/profiles/v2.json \
  --columns 9 --rows 6 --square-mm 25 --views 24 --seconds 90
systemctl --user start orion-vision.service
```

Use the inner corner counts and measured square size of your board. Move and
tilt it across the view, including the edges; at least 12 distinct views are
required. Corner detection and the fisheye fit both run at 320×240 lores.
The CLI saves numerical intrinsics plus `ScalerCrop`, negotiated sensor output
size/bit depth, lores stride and flips to `~/.config/orion/camera-v2.json` and
refuses RMS reprojection error above 0.5 pixels, preserving any previous fit.
Camera-off settings are respected. A low fitting error does not validate the
camera mount or timestamp alignment; check those on the lamp too.
Both profile flips must be false; reflected-ray handling is unsupported.
Refit calibration files that lack geometry. Runtime configures the saved sensor
mode and crop, refuses negotiated sensor/stride mismatches, and drops frames
with a different crop, mode, flips or buffer shape before inference. Changing
the optical stream geometry requires a fresh fit.

One worker runs YuNet at up to 10 Hz idle and 3 Hz from confirmed wake through
reply playback. It polls `voice status`; unavailable voice state uses 3 Hz.
OpenCV uses one thread. The latest lores frame replaces pending work; inference,
YUV conversion, sockets and model loading never run in the camera callback.
ISP padding is removed after packed YUV conversion. No inference frames are
saved or sent off the Pi.

The worker undistorts face box centres to unit camera rays, asks `oriond`
`camera-rays` at the exposure midpoint, then associates base azimuth/elevation
with a constant-velocity Kalman filter and a 20° nearest-neighbour gate. Tracks
expire after one second. IDs increase for the service lifetime, including after
privacy off/on. `looking_at_orion` is a landmark symmetry heuristic for a
frontal face. Selection uses the largest frontal face, otherwise the largest.
Targets and clears go only through runtime `gaze`; enable runtime `--gaze on`
to allow movement. Rejected gaze commands preserve tracks and observations;
the worker polls gaze availability every three seconds before sending targets
again. Only detector or camera-ray failures invalidate tracking. A missed
detection retains the chosen live track until its one-second expiry without
replaying its capture timestamp. Clear is sent once when the target disappears,
or on privacy off. Health reports gaze availability separately from detection
errors. Calibration and motion limits stay owned by the runtime.

`/observations` includes `id`, `class`, `track_id`, base `azimuth`/`elevation`
in radians, `capture_boottime_ns`, observation `age_ms`, `track_age_ms`,
confidence, frontality, normalized `box`, `box_pixels`, five pixel landmarks,
and `image_size`. Health includes detection rate, drops and error. Missing or
invalid intrinsics/model leave streaming available with detection unavailable.
Override paths with `--intrinsics`, `--face-model`, and `--runtime-socket`.

`/speaker` accepts one nonnegative `since_ns` in `CLOCK_BOOTTIME`, rejects
timestamps more than 50 ms ahead, and bounds history to three seconds/32 samples
per track. It returns `speaker` (an observation or null), `strategy`
(`mouth_motion`, `looking_at_orion`, `nearest`, or `none`), the bounded
`since_boottime_ns`, and `observed_boottime_ns`. Only observations younger than
one second participate. Mouth movement is normalized to the eyes and measured
with robust adjacent-frame differences above each track's pre-window jitter
baseline. A clear margin over the second face is required; noisy or ambiguous
evidence falls back to frontality and face area. Area is a distance proxy.
Missing detection resources return 503 `detection_unavailable`; privacy off
returns 503 `camera_off` and purges the mouth history too. See
[wake attention](../docs/voice-architecture.md#direction-evidence) for ownership,
freshness and movement behavior. Multi-person speaker selection needs lamp
validation; mouth-corner jitter and head turns can confuse the heuristic.

Hardware acceptance remains unverified: walk slowly across the room and record
neck-first tracking for review. Repeat identical-audio P0 ASR trials with the
service at these rates under `CPUQuota=60%`/`Nice=10`; the target is at most 10%
ASR slowdown. Check longer thermal load, the fitted lens/mount, capture-time
alignment and camera-off closure/gaze cancellation on the lamp.

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
