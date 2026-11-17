# Third-party notices

## MediaPipe palm HEF for HAILO8L

Orion redistributes `hailo8l/palm_detection_lite.hef` from [AlbertaBeef/blaze_tutorial](https://github.com/AlbertaBeef/blaze_tutorial). The source archive is `blaze_hailo8l_models.zip` from release [`hailo8_version_2`](https://github.com/AlbertaBeef/blaze_tutorial/releases/tag/hailo8_version_2).

The file is unmodified. Orion supplies it as `palm_detection_lite-hailo8l.hef` in the `orion-models-v1` release. [The HEF manifest](vision/orion_vision/hailo-models.json) contains its SHA-256 hash and Orion download URL.

The source project distributes this file under Apache-2.0. [The complete source license](vision/orion_vision/BLAZE-PALM-LICENSE) accompanies Orion's vision package. The CPU palm decoder also retains the [OpenCV Zoo license](vision/orion_vision/PALM-LICENSE).

## Reachy Mini Emotions Library

Orion builds its sound cues in `audio/cues/` from the [Reachy Mini Emotions Library](https://huggingface.co/datasets/pollen-robotics/reachy-mini-emotions-library) by Pollen Robotics.

The files are modified. Each cue is one short part of a library recording with a lower pitch, new fades and a new level. [The cue builder](audio/build_cues.py) lists the source recording for each cue.

The source project distributes the library under Apache-2.0. [The complete source license](audio/REACHY-EMOTIONS-LICENSE) accompanies Orion's audio assets.
