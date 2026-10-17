# Orion hardware versions

`oriond` defaults to V2. The `--hardware v1` option selects V1 profiles and assets.

## Hardware differences

| Item | V1 | V2 |
| --- | --- | --- |
| Lights | 40-pixel RGBW matrix | 24-pixel RGBW ring |
| Light output | GPIO12 | GPIO12 |
| Audio | ReSpeaker 2-Mics V2 HAT | XVF3800 USB array |
| Camera | None | IMX708 Wide |
| Hardware profile | `hardware/profiles/v1.json` | `hardware/profiles/v2.json` |
| Joint configuration | `simulation/mujoco/config/servo_calibration.json` | `hardware/profiles/v2-joints.json` |
| Pose file | `motion/config/v1/poses.yaml` | `motion/config/v2/poses.yaml` |
| Motion directory | `motion/motions/v1/` | `motion/motions/v2/` |
| Scene directory | `scenes/v1/` | `scenes/v2/` |
| Simulation scene | `simulation/mujoco/scene.xml` | `simulation/mujoco/v2/scene.xml` |

V2 stores each motor's zero in EEPROM. EEPROM is memory that keeps its contents without power. The tracked joint profile supplies IDs, directions and limits. V2 does not use a calibration file in the user's configuration directory.

## Calibrate V2 first

New servos need individual ID assignment. An assembled lamp needs mechanical-rest zeroing after servo replacement or horn removal.

WARNING: Powered joints can pinch fingers. Disconnect servo power before changing wiring.

1. Stop the Orion services before servo setup.

   ```bash
   sudo systemctl stop orion-voice-stack orion-vision orion-studio-gateway orion-listener oriond
   ```

2. Connect one new servo to the bus.
3. Assign its joint ID with the corresponding joint name.

   ```bash
   oriond servo set-id --joint shoulder_pitch_joint
   ```

4. Repeat individual assignment for each new servo.
5. Connect all five servos.
6. Fold the lamp into the rest position shown below.

The upper arm lies back on the base. The forearm folds over the upper arm. The shoulder housing supports the head. The head and base face forward.

![V2 rest, side view](images/v2-rest-side.png)
![V2 rest, front view](images/v2-rest-front.png)

CAUTION: An incorrect fold can cause joint damage after zeroing. Keep the lamp still during zeroing.

7. Write the mechanical-rest offsets.

   ```bash
   oriond servo zero
   ```

8. Read the joint positions without enabling torque.

   ```bash
   oriond --check
   ```

All joints should read near zero radians. Zeroing requires two seconds with no movement greater than 10 encoder counts. It checks for final readings of 2047 ±8 counts. A failed write restores the previous offsets.

WARNING: Orion moves toward `home` when its services start. Keep people and objects outside its movement area.

9. Start the Orion services.

   ```bash
   sudo systemctl start oriond orion-listener orion-studio-gateway orion-voice-stack orion-vision
   ```

The commands select one detected USB servo adapter automatically. The `--port` option selects an adapter when several are connected.

## Joint names and limits

| V2 ID | Joint name | Movement |
| --- | --- | --- |
| 1 | `base_yaw_joint` | Base turn |
| 2 | `shoulder_pitch_joint` | Upper-arm tilt |
| 3 | `elbow_pitch_joint` | Forearm tilt |
| 4 | `head_pitch_joint` | Head tilt at the wrist |
| 5 | `head_roll_joint` | Head turn at the neck |

V1 assigns ID 4 to `head_roll_joint` and ID 5 to `head_pitch_joint`.

V2 uses this angle relation:

```text
angle = (raw - 2047) × direction × 2π / 4096
```

The joint profile gives the permitted command range in radians. Startup accepts readings up to 20 counts outside that range. It clamps initial goals into range. Later commands use the strict profile limits.

The runtime rejects ranges that cross the 0/4095 encoder boundary. Range checks cannot identify an incorrectly folded lamp. Mechanical-rest setup remains necessary.

## USB capture and playback

V2 uses the ALSA card name `Array`. The listener owns microphone capture. `oriond` owns speaker playback. Hardware profiles select the default devices.

The installer adds the login user to device groups and installs `voice/99-respeaker-xvf3800.rules`. A reboot applies those permissions. V1 audio setup is in the [audio HAT guide](../hardware/audio/README.md).

## Lights and camera

Both versions use the [GPIO12 lighting driver](../hardware/lighting/README.md). V2 uses the [vision service](../vision/README.md) for camera processing.
