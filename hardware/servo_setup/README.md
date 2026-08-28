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

## Verify the provisioned bus without movement

After all five servos have unique IDs, verify them before commanding movement.
With servo power off, daisy-chain the five labelled servos and connect the end
of the chain to the controller. Keep all horns detached and unloaded. Then turn
servo power on and run:

```bash
uv run orion-verify-servos --port /dev/ttyACM0
```

Replace the example port with the controller port found earlier. Type `VERIFY`
only after checking that IDs 1 through 5 are present in the printed plan.

The verification command is deliberately read-only. It:

1. Opens the bus at LeRobot's standard 1 Mbps baud rate.
2. Pings IDs 1 through 5 and checks that each reports the STS3215 model number.
3. Checks that all connected servos use compatible firmware.
4. Reads raw encoder position, supply voltage, temperature, and torque state.
5. Closes the serial port without writing a torque or configuration register.

The raw encoder position is not yet an Orion joint angle. Mechanical zero,
direction, and safe ranges are established during calibration after assembly.

To preview the verification plan without opening hardware:

```bash
uv run orion-verify-servos --port /dev/not-opened --dry-run
```

If all five pass, switch servo power off before disconnecting the chain. The
next step is a single-servo torque and deliberately small unloaded-motion test.

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
- LeRobot Feetech bus implementation used as a dependency:
  <https://github.com/huggingface/lerobot/blob/main/src/lerobot/motors/motors_bus.py>

Orion's wrapper is implemented specifically for Orion and does not copy the
LeLamp follower runtime. LeRobot is Apache-2.0 licensed; Orion remains
GPL-3.0-only.
