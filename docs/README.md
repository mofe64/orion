# Orion documentation

## Start here

Read these in order when you are new to Orion:

1. [Quickstart](quickstart.md): set up a Pi, connect Studio and deploy updates.
2. [System architecture](system-architecture.md): the processes on the Pi and how they talk.
3. [How Orion moves](motion-architecture.md): from a request to servo movement.
4. [Voice architecture](voice-architecture.md): wake word, transcription, agent and reply.
5. [Hardware versions](hardware-versions.md): V1 and V2 differences, V2 calibration and centring.

## Reference

- [Motion reference](motion-reference.md): pose and motion schemas, styles, animation catalogue, trajectory compiler and servo control
- [Speech animation runtime](speech-animation-runtime.md): how reply audio becomes head and body gestures
- [Configuration](configuration.md): settings files, environment variables and deploy options
- [Runtime commands and lifecycle](../runtime/README.md)
- [Scene format](../scenes/README.md)
- [Motion asset folders](../motion/README.md)
- [Audio cues](../audio/README.md)
- [Robot description](../description/README.md)
- [Train and evaluate “Hey Orion”](wake-word-training.md)
- [Echo cancellation and barge-in design](echo-cancellation-and-barge-in.md) (planned, not implemented)

## Hardware setup

- [STS3215 servo setup, calibration and centring](../hardware/servo_setup/README.md)
- [V1 ReSpeaker HAT audio](../hardware/audio/README.md); V2 USB audio is in [hardware versions](hardware-versions.md#usb-capture-and-playback)
- [RGBW light](../hardware/lighting/README.md)

## Component development

- [Studio](../orion_studio/README.md#development)
- [Runtime build and tests](../runtime/README.md#build-and-test)
- [Runtime in MuJoCo](../runtime/README.md#mujoco-daemon)
- [MuJoCo pose editor](../simulation/mujoco/README.md#calibrated-pose-editor)
- [Orion service](../orion-service/README.md)
- [Voice coordinator](../coordinator/README.md)
- [Agent runtime, memory and tools](../agent/README.md)
- [Pi speech workers and models](../speech/README.md#setup-on-the-pi)
- [Pi voice capture](../voice/README.md#setup)

## Experiment tooling

- [Servo tracking capture, trials and gain experiments](servo-tracking.md)

## Learning notes (V1 model)

- [Joint structure](learning_notes/orion_joints.md)
- [MuJoCo model](learning_notes/orion_mujoco_model_basics.md)
- [URDF basics](learning_notes/orion_urdf_basics.md)
