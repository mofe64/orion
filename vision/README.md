# Orion vision

## Setup on Ubuntu 24.04 / Raspberry Pi 5

The service owns the V2 IMX708 Wide camera and keeps frames in memory. It
serves HTTP on `/tmp/orion-vision.sock`. Built-in wide-lens intrinsics and the
pinned YuNet model let it track faces and send base bearings to runtime gaze
after deployment, without creating a camera intrinsics file.

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
`CPUQuota=150%`, and one OpenMP/OpenBLAS thread. The quota is a runaway ceiling
for the whole service, including camera threads; Nice gives ASR priority under
contention. The Unix socket is user-only
(mode 0600). Stop other camera applications before starting it.

| GET endpoint | Response |
| --- | --- |
| `/health` | Camera state, `tracking_mode`, `camera_restarts`, latest `capture_boottime_ns`, drops and connected streams |
| `/frame.jpg` | Latest main frame resized to 640×360; timestamp in `X-Capture-Boottime-Ns` |
| `/stream` | `multipart/x-mixed-replace`, boundary `orion-frame`, JPEG parts with length and timestamp |
| `/observations` | Recent base-bearing face tracks, normalized boxes, pixel landmarks and capture timestamps |
| `/detections` | Latest preview detection batch; 503 `camera_not_viewed` without a stream or snapshot lease |
| `/speaker?since_ns=…` | Chosen face and selection strategy within a bounded mouth-motion window; 503 `tracking_off` in Off mode |

JPEGs are encoded on demand and cached across viewers at at most 15 fps.
With detection available, preview JPEGs use completed inference captures: up to
10 fps idle and 3 fps during voice activity. With tracking off or detection
unavailable, JPEGs use the
ordinary latest capture. A completed preview pairs its RGB buffer with fresh
detections from that exact capture, so retained gaze tracks never create boxes
on another frame.
Disconnected viewers cause encoding to stop. Capture converts RGB only while
a stream client is connected or a snapshot was requested in the last two
seconds. Callbacks without viewing demand omit the main RGB copy. Tracking
in Faces mode copies the 640×360 lores YUV buffer independently, so detection continues
without full-frame RGB copies. The first viewing request waits up to one second
for an RGB frame. Capture retains one latest RGB buffer; inference retains at
most one pending, one in-flight and one completed preview. They remain in memory
and slow consumers drop frames rather than accumulating a queue. Viewer departure
discards preview pixels without cancelling gaze tracking.
`Picamera2Source` uses 1280×720 `BGR888`, whose Python byte order is RGB.
`make_array("main")` supplies the one owned copy when RGB is requested;
callbacks do not encode or write frames.

`/detections` returns `capture_boottime_ns` as an exact decimal string, `age_ms`
at response creation, and a `detections` array. Each item has a normalized
`box: [x, y, width, height]` in the stream image, `score`, `track_id`,
`chosen_target` and the same timestamp string. A frame with no faces has an
empty array even while gaze retains a track. Requests never create or extend
a viewer lease. Detection batches and boxes are never persisted or logged.

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

The same watcher polls the independent tracking preference. Off clears
pending inference and tracks, discards in-flight results, and reports a zero
detection rate. `/observations` and the viewed `/detections` batch are empty;
`/speaker` returns `503 tracking_off`. Faces resumes with fresh track IDs.
Override its path with `--tracking-preference-file`; keep gateway and Vision
paths aligned. Defaults and failure behaviour are in
[configuration](../docs/configuration.md#camera-privacy-and-profile).

The capture watchdog waits until startup is more than three seconds old, then
reopens the source if no capture has been published for two seconds. It logs
one warning per recovery and increments `camera_restarts`, using the ordinary
start/retry path. Camera toggles retain the count; process restart resets it.
Camera-off never triggers recovery. Old source callbacks cannot publish into
a reopened generation.

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

## Face tracking

Release deployment installs YuNet in `vision/models/` and passes its path through
`--face-model`; rollback selects the previous release's model. The native
installer also supplies the default model for manual service runs.
Both use YuNet 2023mar from OpenCV Zoo commit
`47534e27c9851bb1128ccc0102f1145e27f23f98`. The manual default lives at
`/usr/local/share/orion/vision/face_detection_yunet_2023mar.onnx`. Installers verify
SHA-256 `8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4`
before replacement. The service verifies that hash again before model loading.
OpenCV headless is pinned to 4.11.0.86 in the vision lockfile.

The service configures the IMX708 Wide's full-view 2304×1296 RAW10 sensor mode
and full-sensor `ScalerCrop`, with main 1280×720 RGB and lores 640×360 YUV420.
The lores `SIZE` in [built-in intrinsics](orion_vision/intrinsics.py) also sets
replay dimensions, detector input and observation box normalization. Keeping
`preserve_ar: False` at 16:9 scales the full sensor view without squeezing faces
or adding another aspect-ratio crop. The intrinsics derive an OpenCV fisheye
model from the Camera Module 3 Wide's published focal length, pixel pitch and
horizontal field of view, then apply the configured sensor, crop and lores scales.
This is a nominal lens model; physical mount and timestamp alignment still need
lamp validation. No fitting command or saved intrinsics are required or loaded.

Both profile flips must be false. Startup refuses a negotiated sensor mode or
lores size/format that differs from the requested configuration and records
the ISP's negotiated stride before capture starts. Frames with a different
crop, mode, flips or buffer stride/shape are dropped before inference.

One worker runs YuNet at up to 10 Hz idle and 3 Hz from confirmed wake through
reply playback. It polls `voice status`; unavailable voice state uses 3 Hz.
OpenCV uses one thread. The latest lores frame replaces pending work; inference,
YUV conversion, sockets and model loading never run in the camera callback.
ISP padding is removed after packed YUV conversion. No inference frames are
saved or sent off the Pi.

YuNet uses `FACE_SCORE_THRESHOLD = 0.75` and NMS (non-maximum suppression) at
0.3. Lamp measurements on 2026-10-11 scored a face at about 3 m at 0.59 in
squeezed 320×240 input and 0.91 at 640×360. At about 1 m, scores of 0.900–0.915
were marginal against the previous 0.9 cutoff. The larger 16:9 input preserves
face proportions and more pixels; the lower cutoff leaves detection headroom.

The worker undistorts face box centres to unit camera rays, asks `oriond`
`camera-rays` at the exposure midpoint, then associates base azimuth/elevation
with a constant-velocity Kalman filter and a 20° nearest-neighbour gate. Tracks
expire after one second. IDs increase for the service lifetime, including after
privacy off/on. `looking_at_orion` is a landmark symmetry heuristic for a
frontal face. Initial gaze selection uses the largest frontal face, otherwise
the largest. A fresh current target stays selected despite small area changes.
Another frontal face must maintain at least 1.5 times its box area for one
second to replace it; losing the target allows immediate reselection. Missing
challenger detections or loss of that advantage resets the interval. This
hysteresis affects gaze only; speaker selection keeps its mouth-motion,
frontality and area ordering.
Targets and clears go only through runtime `gaze`. V2 servers enable gaze by
default; `--gaze off` disables it, and movement still requires powered character
mode. Stop reports `state:"stopped"` and the originating `stop_command`. A confirmed
wake can clear Stop only on a powered, enabled character. Disable needs explicit
character start, or power restoration followed by scripted `gaze resume`.
Returning vision cannot clear either latch. See [runtime recovery](../runtime/README.md#bearing-gaze-v2).
Missing runtime camera geometry reports gaze unavailable without stopping the
daemon. Rejected gaze commands preserve tracks and observations;
the worker polls gaze availability every three seconds before sending targets
again. Only detector or camera-ray failures invalidate tracking. A missed
detection retains the chosen live track until its one-second expiry without
replaying its capture timestamp. Clear is sent once when the target disappears,
or on privacy or tracking off. Health reports gaze availability separately from detection
errors. Calibration and motion limits stay owned by the runtime.

`/observations` includes `id`, `class`, `track_id`, base `azimuth`/`elevation`
in radians, `capture_boottime_ns`, observation `age_ms`, `track_age_ms`,
confidence, frontality, normalized `box`, `box_pixels`, five pixel landmarks,
and `image_size`. Health includes detection rate, drops and error. A missing or
invalid detector model leaves streaming available with detection unavailable.
Override paths with `--face-model` and `--runtime-socket`.

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

After deployment, check the negotiated lores stride and YuNet's per-frame
median/p95 time. On the lamp, compare reply latency on comparable spoken
utterances with Tracking Faces (10 Hz idle, 3 Hz during voice activity) and
Tracking Off, keeping capture and viewing conditions matched. Use the installed
quota and Nice settings above and target at most 10% slowdown; record ASR
latency separately when available.
Use the [fixed-audio ASR timing trial](#fixed-audio-asr-timing-trial) for the
matched ASR comparison.
If that comparison fails, reduce `SIZE` to `(480, 270)` and repeat the checks.
At 1, 2 and 3 m, confirm non-empty `/observations` and a stable track ID while
standing still for 30 seconds. Check that the neck follows a slow side-to-side
walk, then point the camera at the TV and posters for one minute and confirm
there are no false faces. These checks need physical lamp validation. Also
check longer thermal load, the fitted lens/mount, capture-time alignment and
camera-off closure/gaze cancellation.

Verify Off removes boxes and following and drops detector CPU, then Faces
resumes detection. With two people standing still, confirm gaze keeps one
target. Keep the Studio camera open for ten minutes, toggling camera capture
off and on once. Check advancing `capture_boottime_ns`, `camera_restarts: 0`,
and the service cgroup's `cpu.stat` `nr_throttled`; record temperature after
ten minutes idle with the camera on.

The 2026-10-11 lamp check of release `bf05dcd720e5-1f0261b6` passed a 600 s
Studio-viewed run at 640×360, `CPUQuota=150%` and `Nice=10`. Capture timestamps
and inference counts advanced, `camera_restarts` remained zero, and
`nr_throttled` stayed at 1 with no added throttling time. Temperature rose from
51.8°C to 59.5°C. Studio recovered after the intentional camera off/on toggle.
Mofe also confirmed the Tracking Off/Faces, steady two-person gaze and matched
ASR comparison acceptance checks passed. Numeric ASR timing samples were not
supplied with that confirmation.

### Fixed-audio ASR timing trial

Run `vision/scripts/asr_timing.py` on the Linux Pi with the installed Qwen
assets. It starts its own `llama-server` through the production speech adapter,
using the same flags, a random loopback port and a private key. It does not
connect to, stop or reconfigure the live speech server. Model loading and one
warm-up transcription are excluded; all timed runs use identical PCM bytes.
It terminates its own child on completion, inference failure or interruption.
The native launcher also binds the child's lifetime to the benchmark process.

Create one spoken fixture outside the checkout. For V2's two-channel USB
capture, select processed channel 0 and convert it to PCM16 mono at 16 kHz
(`arecord` and `ffmpeg` must be installed). The listener holds the microphone;
stop it for the recording and restart it before timing. These commands assume
it was running; leave an intentionally stopped listener stopped.

```bash
mkdir -p "$HOME/.cache/orion/asr-trial"
sudo systemctl stop orion-listener
# Speak a short command during this five-second recording.
arecord -D plughw:CARD=Array,DEV=0 -f S16_LE -r 16000 -c 2 -d 5 \
  "$HOME/.cache/orion/asr-trial/source.wav"
sudo systemctl start orion-listener
ffmpeg -y -i "$HOME/.cache/orion/asr-trial/source.wav" \
  -af 'pan=mono|c0=c0' -ar 16000 -c:a pcm_s16le \
  "$HOME/.cache/orion/asr-trial/fixture.wav"
```

The tool rejects stereo, non-PCM16, empty, truncated or longer-than-33-second
audio. It prints no transcript and disables the adapter's optional diagnostic
audio capture. Keep the same fixture for both conditions; its SHA-256 and
duration in each result identify the input.

With Tracking Off in Studio, run from the checkout:

```bash
python3 vision/scripts/asr_timing.py \
  --wav "$HOME/.cache/orion/asr-trial/fixture.wav" \
  --model "$HOME/.local/share/orion/voice-stack/models/qwen" \
  --llama-server "$HOME/.local/share/orion/voice-stack/llama-b10976/llama-server" \
  --runs 20 --label tracking-off \
  > "$HOME/.cache/orion/asr-trial/tracking-off.json"
```

Choose Tracking Faces, then repeat with `--label tracking-faces` and output
`tracking-faces.json`. Keep camera capture, Studio viewing, cooling, ASR thread
count and fixture identical. Avoid live voice requests during the trial: the
separate benchmark process does not mark a runtime voice session active, so
Faces uses its idle detection rate. This measures ASR contention from idle
tracking; voice-session 3 Hz behavior still needs a conversation check.
`ORION_ASR_THREADS` defaults to the production adapter's three threads.
`ORION_ASR_MODEL_DIR` and `ORION_LLAMA_SERVER` can supply the asset paths when
the corresponding options are omitted.

Stdout is one JSON result containing `median_ms`, `p95_ms`, `samples_ms`, the
run count and fixture identity. The p95 uses nearest rank (ceil(0.95 × N));
use at least 20 timed runs for the comparison. Diagnostics use stderr and
failure returns `ok: false` with a nonzero exit status. Calculate slowdown as
`100 × (faces_median_ms / off_median_ms - 1)` and compare p95 separately.
Repeat the pair in reverse order to check thermal drift. Target at most 10%
median slowdown. The acceptance confirmation is recorded above; retain both
JSON files when repeating the measurement.

### Capture timing record

An in-memory Pi probe on 2026-10-11 negotiated a 640-byte lores stride and a
540×640 packed YUV buffer for `SIZE = (640, 360)`, with `preserve_ar: False`.
The candidate detector used the pinned model, a 0.75 score cutoff, one OpenCV
thread, the previous 60% CPU quota and `Nice=10`. This historical measurement
predates the quota change and capture watchdog. After three warmup inferences, each phase
ran for 12 seconds with capture continuing at 30 fps:

| Requested detection rate | Measured rate | Inferences | Median time | p95 time |
| --- | --- | --- | --- | --- |
| 10 Hz | 7.92 Hz | 95 | 113.18 ms | 168.97 ms |
| 3 Hz | 3.00 Hz | 36 | 59.90 ms | 90.76 ms |

Times include YUV conversion and detection and measure wall time under the
CPU cap. Each phase reused its initial latest frame once; other inferences
sampled fresh captures. The 10 Hz phase exceeds the 100 ms frame budget.
This probe establishes geometry and bounded-workload timing; ASR slowdown,
distance tracking and false-face acceptance still need the manual lamp checks.

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

This standalone benchmark retains main 1280×720 RGB and lores 320×240 YUV
streams at a requested 30 fps. Its timings do not measure the service's
640×360 detector input or its 10/3 Hz rate limits. Five equal timed windows
measure capture alone, YuNet at one and two threads, then YOLO at one and two
threads. Startup, model loading,
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

Measure Qwen ASR latency with the [fixed-audio trial](#fixed-audio-asr-timing-trial)
while the camera benchmark is off and running. `measure.py --label voice`
identifies the workload; it does not instrument Qwen.
The JSON's `qwen_asr_slowdown` remains `null` until that separate comparison
is recorded. Include capture rate, inference median/p95, ASR slowdown,
temperature, cable continuity and orientation in the commissioning results.
These hardware results are required before selecting detector frame rates.
