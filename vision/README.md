# Orion vision

`orion-vision` owns V2 camera capture and local detection. It serves JPEG images and observations through `/tmp/orion-vision.sock`. Captured frames stay in memory.

## Setup on Raspberry Pi OS

The [release installer](../docs/quickstart.md) supplies camera runtime libraries and Python 3.12 wheels for Bookworm or Trixie. `scripts/release-native.json` selects matching camera, KMS and Hailo packages. Package holds protect those versions from ordinary OS upgrades.

The camera profile is `hardware/profiles/v2.json`. The service uses an IMX708 Wide camera. Privacy and tracking preferences are separate; see [configuration](../docs/configuration.md#camera-privacy-and-tracking-profile).

## AI HAT+ setup

The HAT is optional. Faces, objects and palms can run on HAILO8L. Each detector has a CPU fallback. See the [vision reference](../docs/vision-reference.md) for models, rates and accelerator settings.

## Service setup and development

Run source commands from the repository root. Real Pi capture requires the native wheels and libraries supplied by the release installer.

1. Install locked Python dependencies for local development.

   ```bash
   uv sync --project vision --locked --managed-python
   ```

2. Run the vision tests.

   ```bash
   uv run --project vision --locked --managed-python python -m unittest discover -s vision/tests -v
   ```

3. Run replay with synthetic JPEG fixtures.

   ```bash
   uv run --project vision --locked --managed-python orion-vision --profile hardware/profiles/v2.json --replay /path/to/synthetic-fixtures --socket /tmp/orion-vision-test.sock
   ```

Replay files must contain synthetic inputs. Camera capture does not write frames to disk.

## Capture and inference

Camera callbacks copy buffers and metadata. JPEG encoding and inference run outside those callbacks. Each worker uses the latest frame and drops older work.

Capture timestamps use exposure-midpoint `CLOCK_BOOTTIME`. Runtime `camera-rays` supplies the measured capture-time transform. This keeps a target's world direction stable while the lamp moves.

One worker schedules face, palm and object work. The CPU path uses YuNet, MediaPipe palms and YOLO11n through NCNN. The Hailo path shares one child process for all three networks. Both paths keep model execution outside camera callbacks.

The [model reference](../docs/vision-reference.md#model-installation) links the download manifests. The dependency lock selects NumPy and headless OpenCV versions. Model licenses remain with their packages.

## Tracking modes

| Mode | Background behavior |
| --- | --- |
| Off | Plain viewing without background detection |
| Idle | Observations for enabled behaviors and conversation speaker following |
| Faces | Face detection and ambient following |
| Hands | Palm detection and work-area following |

Mode changes invalidate pending results. Camera privacy off closes capture in every mode. Neither hand presence nor a tracking selection enables torque or wakes a resting lamp.

Gaze requests go through the runtime. The service never writes servo goals. Stop, rest, foreground work and runtime ownership rules can reject movement.

## Local interfaces

The gateway authenticates external camera requests. The vision socket remains local to the Pi user.

| Interface | Function |
| --- | --- |
| `/stream` | Latest in-memory JPEG stream |
| `/frame.jpg` | One JPEG |
| `/observations` | Local observation metadata |
| `/detections` | Preview detection boxes |
| `/speaker` | Confirmed-session speaker selection |
| `/find` | Explicit object lookup |

Face boxes require the same capture timestamp as the displayed JPEG. Studio removes stale boxes. Saved snapshots contain the image without detection overlays.

Explicit object lookup uses the same bounded worker. Desk points assume an intersection with the base-frame desk plane. One camera cannot establish that an object rests on that plane.

## Troubleshooting

1. Read recent service messages.

   ```bash
   journalctl -u orion-vision -n 60 --no-pager
   ```

2. Read service status.

   ```bash
   systemctl --no-pager --full status orion-vision
   ```

Camera-off closes capture and clears image buffers. A capture watchdog reopens a stalled source. Camera viewing also requires permission from the runtime's rest coordinator.
