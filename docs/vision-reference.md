# Vision detectors

The Raspberry Pi AI HAT+ is optional. Orion uses CPU detectors when the lamp has no HAT. Builders do not need a HAT.

## Detector backends

A backend is the program that runs a detector. One child process owns the Hailo device and all three networks. Requests contain one frame at a time. Camera callbacks do not run inference. Captured frames stay in memory.

| Detector | Hailo backend | CPU backend |
| --- | --- | --- |
| Faces | SCRFD 2.5G, with boxes and five landmarks | YuNet |
| Objects | YOLOv11n, with the existing COCO class names | YOLO11n through NCNN |
| Palms | MediaPipe palm detection lite on HAILO8L | MediaPipe palm detection |

A HEF is a compiled network file for a Hailo device. Face and object HEFs come from Hailo Model Zoo v2.14 for HailoRT 4.20. Each device architecture has separate files. Orion selects `HAILO8` or `HAILO8L` from the `Device Architecture:` field.

The [face model list](https://github.com/hailo-ai/hailo_model_zoo/blob/v2.14/docs/public_models/HAILO8L/HAILO8L_face_detection.rst) supplies SCRFD 2.5G. The [object model list](https://github.com/hailo-ai/hailo_model_zoo/blob/v2.14/docs/public_models/HAILO8L/HAILO8L_object_detection.rst) supplies YOLOv11n. Both files are precompiled. The Pi does not compile networks.

The HAILO8L palm HEF comes from `blaze_tutorial`, release `hailo8_version_2`. Orion hosts this precompiled file in its own model release. The [third-party notices](../THIRD_PARTY_NOTICES.md) contain its source and Apache-2.0 license. HAILO8 has no supplied palm HEF and uses CPU palm detection.

NumPy prepares network inputs and restores frame coordinates. The HEFs include input normalization. SCRFD box decoding, landmark decoding and overlapping-box removal run in NumPy. HailoRT runs YOLO box decoding and overlapping-box removal on the CPU. NumPy converts YOLO coordinates, maps class names and filters results. Only network inference runs on the HAT.

Palm detection uses the same NumPy anchor decoding and overlapping-box removal on both backends. Hailo returns raw palm regression values and scores. The shared decoder applies the palm score threshold and restores frame coordinates.

`vision/orion_vision/detector_policy.py` defines backend rates and score thresholds. SCRFD uses its own probability threshold. It does not use YuNet's threshold.

| Work | Hailo detector | CPU detector |
| --- | --- | --- |
| Faces, idle or voice activity | 15 Hz | 10 Hz idle; 3 Hz during voice activity |
| Palms, Hands mode | 15 Hz on HAILO8L | 10 Hz idle; 3 Hz during voice activity |
| Background objects | Every 5 seconds | Every 60 seconds |

Background object work requires an enabled object behavior and runtime permission. Hands mode runs the palm detector instead of face tracking.

## Accelerator setting

`~/.config/orion/vision.json` stores `vision.accelerator`. A missing file selects `auto`. The service reads the file at startup.

```json
{"vision": {"accelerator": "auto"}}
```

| Value | Behavior |
| --- | --- |
| `auto` | Use Hailo when the device and its matching HEFs are available; otherwise use the CPU |
| `hailo` | Request Hailo; report an unavailable device or model and use the CPU |
| `cpu` | Use the CPU without device discovery |

HAILO8 palms use the CPU for all three values. Invalid settings select the CPU. `--accelerator` overrides the saved setting. `--settings-file` selects another settings file.

1. Set `vision.accelerator` to `cpu` to force CPU inference.
2. Restart vision after changing the setting.

   ```bash
   sudo systemctl restart orion-vision
   ```

3. Read the backend names from the local interface.

   ```bash
   curl --silent --unix-socket /tmp/orion-vision.sock http://localhost/health | jq '.detection.backends'
   ```

A HAILO8L lamp can return this object:

```json
{"faces": "hailo", "palms": "hailo", "objects": "hailo"}
```

An accelerator error switches only the affected detector to its CPU backend. The service logs one line for that switch and continues. Its rate changes to the CPU rate. Restart vision to try Hailo again. Camera privacy and runtime movement ownership apply to both backends.

## Model installation

The [release installer](quickstart.md#install-orion) installs Hailo runtime packages only when it finds a Hailo PCIe device. It downloads the matching HEFs into `/usr/local/share/orion/vision/hailo/`. Only HAILO8L installations download a palm HEF.

[The HEF manifest](../vision/orion_vision/hailo-models.json) contains the URLs and SHA-256 hashes. [The CPU model manifest](../scripts/release-models.json) contains the fallback models. Installation and reuse require matching hashes. Vision also checks HEF hashes before it starts the child process.

If device identification needs a reboot, installation leaves vision on the CPU. Run the same installer after reboot to fetch the HEFs. Repeated installation can add these shared files without replacing the active release.

Hailo packages and HEFs are optional for local development and tests. The vision wheel imports HailoRT only inside the accelerator child process.
