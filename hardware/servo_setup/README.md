# Orion STS3215 Servo Setup

This tool performs the first hardware operation in Orion's physical-prototype
milestone: assigning one persistent bus ID to each STS3215 servo.

It is an Orion-native equivalent of LeLamp's `lelamp.setup_motors` workflow.
The implementation uses LeRobot's supported `FeetechMotorsBus` API rather than
copying LeLamp's follower robot class into Orion. This keeps one-time hardware
provisioning separate from Orion's motion package and future `ros2_control`
hardware adapter.

## Why setup is necessary

STS3215 servos share one serial bus. Commands contain a numeric destination ID
so that only the intended servo responds. New servos commonly have the same
factory ID. If several identically addressed servos are daisy-chained, Orion
cannot reliably command or read them individually.

Setup writes two persistent values to each servo:

1. Orion's unique ID for that joint.
2. LeRobot's standard bus baud rate.

The values remain in the servo after power is removed, so normal setup is done
once. Re-run it only when replacing a servo or intentionally changing the ID
map.

Setup is not calibration and does not command movement:

- **Setup** identifies each physical servo on the shared bus.
- **Calibration** later measures its zero offset, direction, and safe range in
  the assembled mechanism.
- **Runtime control** later sends validated positions through Orion's
  `ros2_control` hardware adapter.

## Authoritative ID map

| Orion joint | Joint reference name | Servo ID |
| --- | --- | ---: |
| `base_yaw_joint` | `base_yaw` | 1 |
| `shoulder_pitch_joint` | `base_pitch` | 2 |
| `elbow_pitch_joint` | `elbow_pitch` | 3 |
| `head_roll_joint` | `wrist_roll` | 4 |
| `head_pitch_joint` | `wrist_pitch` | 5 |

The tool programs IDs 5 through 1 so the common factory-default ID 1 remains
until the final step. The joint-to-ID mapping, not the programming order, is
what matters after setup.

## Safety requirements

- Work on a bench before installing servo horns or loading a joint.
- Use the voltage and polarity specified for your exact controller and servo.
- Keep the horn unloaded and keep hands clear whenever servo power is on.
- Connect exactly one unconfigured servo to the controller at a time.
- Turn servo power off before connecting or disconnecting a servo.
- Do not disconnect USB or motor power while an ID write is in progress.
- Do not daisy-chain all five servos until every servo has a unique ID and a
  physical label.

The program requires the exact confirmation word `PROGRAM` before each
persistent write.

## Install the isolated tool

From the Orion repository:

```bash
cd hardware/servo_setup
uv sync
```

The local `.python-version` asks `uv` to use Python 3.12 because the current
LeRobot package requires Python 3.12 or newer and is not yet declared compatible
with Python 3.14.

## Find the controller port

Connect the servo controller to the computer by USB, then run:

```bash
uv run lerobot-find-port
```

The utility asks you to unplug and reconnect the controller. Typical ports are:

- macOS: `/dev/tty.usbmodem...`
- Linux: `/dev/ttyACM0`

## Preview without touching hardware

```bash
uv run orion-setup-servos --port /dev/ttyACM0 --dry-run
```

Dry-run mode does not import LeRobot, open the serial port, or write a servo.

## Program the five servos

Replace the example port with the value found on your computer:

```bash
uv run orion-setup-servos --port /dev/ttyACM0
```

Follow every power-off, single-servo connection, and labelling prompt. If a
step fails or a servo is replaced, retry only that joint:

```bash
uv run orion-setup-servos \
  --port /dev/ttyACM0 \
  --joint elbow_pitch_joint
```

After all five succeed, leave them disconnected until the next bench-test step:
read ID and position, enable/disable torque, and command a deliberately small
unloaded movement.

## Audit the provisioned bus without movement

With the five servos connected and powered, run the read-only register audit:

```bash
uv run orion-verify-servos --port /dev/ttyACM0
```

Replace the example port with the controller port found earlier. The command
opens the bus immediately; it does not prompt or write servo registers. It:

1. Opens the bus at LeRobot's standard 1 Mbps baud rate.
2. Pings IDs 1 through 5 and checks that each reports the STS3215 model number.
3. Reports each servo's firmware version so differences remain visible.
4. Reads firmware, calibration, PID, acceleration, velocity, torque, protection,
   runtime, and live telemetry registers.
5. Prints a compact live-state table and a raw register matrix for comparison.
6. Closes the serial port without writing a torque or configuration register.

The raw encoder position is not yet an Orion joint angle. Mechanical zero,
direction, and safe ranges are established during calibration after assembly.

To list the selected bus IDs without opening hardware:

```bash
uv run orion-verify-servos --port /dev/not-opened --dry-run
```

## Archived one-off physical motion tools

The first-motion nudge and direct named-pose commands were commissioning tools,
not runtime control. Their installed entry points have been removed and their
source now lives in `orion_servo_setup/archived/`. New physical movement will
run through the C++ `ros2_control` hardware interface.

## Calibrate all five joints in one session

The torque-off calibration command remains available as a setup utility:

```bash
uv run orion-calibrate-servos --port /dev/ttyACM0
```

Start with the 6 V servo supply off and use padded blocks to support the arm and
head. At the first prompt, turn 6 V on and type `CALIBRATE ALL`. The supply must
be on for encoder communication, but torque stays off for the complete session;
the command never sends a goal position.

The workflow has one neutral capture and one combined recording window:

1. Put the assembled lamp in the reference LeLamp zero/middle pose and capture
   that position.
2. Start range recording.
3. Move one joint at a time slowly through its usable travel while keeping the
   other links supported.
4. Stop before collision, cable tension, or a hard stop. Never force the
   gearbox.
5. For `base_yaw_joint` (ID 1) and `head_roll_joint` (ID 4), move only about 90
   degrees clockwise and 90 degrees counter-clockwise from neutral. This is the
   original LeLamp cable-protection rule for its two yaw/roll joints.
6. After all five joints have been swept, press Enter once to finish.

The live line displays `minimum/maximum` raw displacement from neutral for each
ID. The command rejects a joint that was barely moved or a range that looks
like a continuous rotation. It subtracts 20 raw steps (about 1.76 degrees) from
both measured endpoints to make software safety limits. If ID 1 or ID 4 was
swept beyond the reference yaw window, the measurement is retained but its
commandable software range is capped at +/-1004 raw steps (about +/-88.2
degrees). The command warns and continues instead of discarding the complete
five-joint session.

The default output is:

```text
~/.config/orion/servo_calibration.json
```

The file contains the neutral encoder value, measured range, reduced safe
range, encoder direction, and equivalent LeRobot homing/range values for every
joint. Existing output is preserved as a timestamped backup before replacement.

### Original LeLamp behavior and Orion's deliberate deviation

Original LeLamp calls LeRobot's `set_half_turn_homings()`, records encoder
ranges, and writes the homing offset and min/max values persistently to each
servo. It uses `drive_mode=0` for all five motors.

Orion records the same measurements in a versioned JSON file without changing
servo EEPROM. It calculates movement as a circular displacement from neutral,
so a joint crossing raw encoder zero is not mistaken for an almost-complete
turn. The stored `encoder_direction=1` preserves LeLamp's `drive_mode=0`
convention for this mechanically compatible build. That sign still needs to be
checked against Orion's URDF joint-positive convention when the physical
`ros2_control` adapter is introduced; full trajectories must not run before
that check.

To preview the complete plan without opening hardware or writing a file:

```bash
uv run orion-calibrate-servos --port /dev/not-opened --dry-run
```

On completion or any error, the command performs a best-effort torque-off for
all five servos. Turn the 6 V supply off after it exits.

## Capture a mechanically stable rest pose

```bash
uv run orion-capture-rest --port /dev/ttyACM0
```

The capture runs with servo torque off. Over a clear padded area, manually put
Orion into a low, balanced arrangement that remains upright without blocks or
hands. The command observes all five encoders for five seconds, rejects more
than 10 raw steps (about 0.88 degrees) of drift, checks the pose against both
the measured hardware calibration and the shared ROS operational ranges, then
requires the exact `SAVE REST` confirmation. A successful capture atomically
adds `rest` to `orion_motion/config/poses.yaml`; use `--replace` only when
deliberately recapturing an existing rest pose. Servo EEPROM is never changed.

This test demonstrates short-term stability in the captured environment; it
cannot certify stability after moving the base, changing payload or cable
routing, or changing the lamp's surface or orientation. Recapture or retest
after any such change. The former direct named-pose executor is archived and
must not be used in place of `ros2_control`.

## How the write works

For each joint, LeRobot's `setup_motor()`:

1. Opens the serial port without assuming an existing ID.
2. Scans supported baud rates for the single connected STS3215.
3. Checks that the responding model is an STS3215.
4. Disables torque so persistent configuration can be changed.
5. Writes Orion's target ID.
6. Writes the bus's standard baud rate.

The Orion wrapper adds semantic names, an explicit one-servo safety prompt,
physical-labelling reminders, cancellation, dry-run mode, and a one-joint retry
path.

## Sources and provenance

- LeLamp setup procedure and ID layout:
  <https://github.com/humancomputerlab/LeLamp/blob/master/docs/2.%20Servos%20Setup.md>
- LeLamp runtime behavior used as the reference:
  <https://github.com/humancomputerlab/lelamp_runtime/blob/main/lelamp/follower/lelamp_follower.py>
- LeLamp assembled-lamp calibration and protected-yaw instructions:
  <https://github.com/humancomputerlab/LeLamp/blob/master/docs/5.%20LeLamp%20Control.md>
- LeRobot Feetech bus implementation used as a dependency:
  <https://github.com/huggingface/lerobot/blob/main/src/lerobot/motors/motors_bus.py>

Orion's wrapper is implemented specifically for Orion and does not copy the
LeLamp follower runtime. LeRobot is Apache-2.0 licensed; Orion remains
GPL-3.0-only.
