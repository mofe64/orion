# Orion hardware versions

Use `--hardware v1` or `--hardware v2` to match your robot. This selects its
lights, audio devices, calibration file, poses, movements and scenes.

<a id="select-the-hardware-profile"></a>

## Hardware differences

The profiles are saved in `hardware/profiles/v1.json` and
`hardware/profiles/v2.json`.

| Hardware or file | V1 | V2 |
| --- | --- | --- |
| Lights | 40-pixel RGBW matrix | 24-pixel RGBW ring |
| Light connection | GPIO12 on the Pi 5 | Same GPIO12 driver |
| Audio | ReSpeaker 2-Mics V2 HAT, `seeed2micvoicec` | XVF3800 USB, `Array` |
| Camera | No camera service | IMX708 Wide camera, managed by `orion-vision` |
| Calibration | `servo_calibration.json` | `servo_calibration-v2.json` |
| Poses | `motion/config/v1/poses.yaml` | `motion/config/v2/poses.yaml` |
| Movements | `motion/motions/v1/` | `motion/motions/v2/` |
| User poses | `motion/user/poses/v1/` | `motion/user/poses/v2/` |
| Scenes | `scenes/v1/` | `scenes/v2/` |
| Simulation | `simulation/mujoco/scene.xml` | `simulation/mujoco/v2/scene.xml` |

Calibration files are stored in `~/.config/orion/` on the onboard computer.
The physical robot, Studio and simulation use the matching pose and movement
files.

## Calibrate V2 first

Run from the Orion checkout on the onboard computer, with the installed runtime
stopped. Follow the [servo setup safety instructions](../hardware/servo_setup/README.md#safety-requirements)
and replace `/dev/ttyACM0` with your servo controller's serial port.

The commands below perform these steps in order:

1. **Verify** checks the five servos' settings and health.
2. **Calibrate** records each joint's zero position and movement limits with the
   motors unpowered.
3. **Centre** adjusts each servo's stored position offset so its movement range
   stays within the positions it can command, from 0 to 4095.
4. **Capture rest** saves a position Orion can hold with the motors unpowered,
   without hands or blocks supporting it. The tool checks that it stays still
   before saving.

```bash
uv sync --project hardware/servo_setup --locked
uv run --project hardware/servo_setup orion-verify-servos --hardware v2 \
  --port /dev/ttyACM0
uv run --project hardware/servo_setup orion-calibrate-servos --hardware v2 \
  --port /dev/ttyACM0
uv run --project hardware/servo_setup orion-centre-servos --hardware v2 \
  --port /dev/ttyACM0
uv run --project hardware/servo_setup orion-capture-rest --hardware v2 \
  --port /dev/ttyACM0
```

Calibration saves `~/.config/orion/servo_calibration-v2.json`. Rest capture
updates the `rest` pose in `motion/config/v2/poses.yaml`. Commit and push that
pose change before deploying an update; deployment replaces the built-in pose
file. The calibration file stays on the onboard computer.

`oriond` rejects calibration ranges that cross the servo's 0/4095 position
boundary. See [servo centring](../hardware/servo_setup/README.md#centre-the-servos-when-a-range-crosses-raw-04095)
for the detailed procedure. Check other poses and movements on the assembled
robot before using them.

## Joint names and directions

V2's joints run from the base through the shoulder, elbow, wrist and neck.
Commands and pose files keep the existing software joint names:

| V2 servo ID | Software joint name | What it moves |
| --- | --- | --- |
| 1 | `base_yaw_joint` | Turns the base |
| 2 | `shoulder_pitch_joint` | Tilts the upper arm |
| 3 | `elbow_pitch_joint` | Tilts the forearm |
| 4 | `head_pitch_joint` | Tilts the head at the wrist |
| 5 | `head_roll_joint` | Turns the head at the neck |

On V1, servo 4 controls `head_roll_joint` and servo 5 controls
`head_pitch_joint`. V2 uses the assignments above. `oriond` rejects a calibration
whose servo IDs do not match the selected hardware profile.

With `encoder_direction: 1`, V2 moves in these directions:

| Software joint name | Negative angle | Positive angle |
| --- | --- | --- |
| `base_yaw_joint` | Turns to the lamp's left | Turns to the lamp's right |
| `shoulder_pitch_joint` | Leans back | Leans forward |
| `elbow_pitch_joint` | Raises the forearm | Lowers the forearm |
| `head_pitch_joint` | Tilts the head up | Tilts the head down |
| `head_roll_joint` | Turns the head to its left | Turns the head to its right |

Calibration defaults to `encoder_direction: 1`. If a fitted joint moves in the
opposite direction, use `--directions FILE` during calibration to set that
joint to `-1`. Joint angles are in radians; model distances are in metres.

Read calibrated hardware state without enabling the motors:

```bash
runtime/target/release/oriond --hardware v2 --check --port /dev/ttyACM0
```

For supervised movement tests, follow the [hardware lifecycle](../runtime/README.md#physical-hardware)
and start with `--serve --hardware v2 --character-on-start off`.
V2's elbow setting still needs physical validation: check that each
`lean_reach_*` pose finishes and that the elbow does not hum or oscillate.
Motor settings are in [runtime/src/devices/sts3215/driver.rs](../runtime/src/devices/sts3215/driver.rs).

<a id="led-and-usb-audio-setup"></a>

## Lights, audio and camera

The V2 ring uses the same [GPIO12 light driver](../hardware/lighting/README.md#persistent-raspberry-pi-5-setup)
as V1. After installing it, reboot and run
`hardware/lighting/verify-persistent.sh`.
Use `--hardware v2` with `oriond` when checking the ring or USB audio.

### USB capture and playback

V2 records two audio channels from the XVF3800 and uses processed channel 0,
which contains the board's cleaned microphone audio. For different firmware
layouts, the listener accepts `--capture-channels 2|6 --processed-channel N`.
See the [Seeed USB guide](https://wiki.seeedstudio.com/respeaker_xvf3800_introduction/).

Microphone direction readings are optional; see [listener settings](configuration.md#microphone-listener).
To read them, install the [XVF3800 USB access rule](../voice/README.md#measure-the-xvf3800-on-v2),
add the account running Orion to `plugdev` with `sudo usermod -aG plugdev USER`,
and restart the listener. Replace `USER` with that account's name. Recording
can work without this permission even when direction readings fail. Release
installation does not perform this setup.

`oriond` sets both USB playback volume controls to 0 dB at startup. To set them
by hand:

```bash
amixer -c Array sset PCM,0 0dB unmute
amixer -c Array sset PCM,1 0dB unmute
```

### Camera

V2 runs `orion-vision` to capture camera images and detect faces and objects.
Set up the camera's libraries before deploying; see [vision setup](../vision/README.md#setup-on-ubuntu-2404--raspberry-pi-5).
The V2 release includes the service. Studio's camera setting controls whether
it captures images. V1 has no camera service.

<a id="cad-urdf-and-mujoco"></a>

## Simulation

V2 uses `description/urdf/orion-v2.urdf` as its robot model and
`simulation/mujoco/v2/scene.xml` for MuJoCo simulation. Studio and simulation
use the same joint angles as the calibrated robot. Prepare the
[simulator environment](../simulation/mujoco/README.md#python-environment)
and build the runtime before starting a V2 preview:

```bash
runtime/target/release/oriond --serve --backend mujoco --hardware v2 \
  --character-on-start on
```

The preview shows movement with a fixed base. It does not prove balance,
collision clearance or how well the real motors follow a movement. The older
standalone pose editors assume V1 geometry.

For model work:

| File | What it contains |
| --- | --- |
| [simulation/mujoco/config/model_reference-v2.json](../simulation/mujoco/config/model_reference-v2.json) | The measured alignment between the model and the robot's calibrated zero positions. |
| `simulation/mujoco/v2/mass-properties.json` | Estimated part masses and weight distribution. |
| `simulation/mujoco/v2/mass-inputs.example.json` | An example file for supplying measured part weights and additional components. |

To rebuild the model or update its mass estimates:

```bash
.venv/bin/python scripts/import_v2_model.py
.venv/bin/python scripts/estimate_v2_inertias.py --inputs /path/to/mass-inputs.json
```

See the [robot description](../description/README.md#v2-camera-and-light-frames)
for model alignment and camera coordinates.

## Deploy the selected version

After calibration, centring and supervised movement checks, commit and push
any pose or movement changes. Deploy with `--hardware v2` using the
[Pi deployment procedure](quickstart.md#deploy-to-the-pi).
The installer checks calibration and compiles movements against its limits
before switching services.

Installed V1 and V2 services start Orion's automatic movement and light
behaviours. When starting `oriond` by hand, V1 defaults to `on` and V2 defaults
to `off`. Use `--character-on-start on|off` to choose explicitly. See
[runtime startup settings](configuration.md#customising-the-oriond-runtime).
