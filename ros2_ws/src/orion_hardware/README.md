# Orion STS3215 ros2_control hardware

`orion_hardware` is the physical backend behind Orion's existing
`joint_trajectory_controller` interface. It does not define poses or
trajectories. Its responsibilities are narrower:

- open the five-servo STS3215 bus;
- validate the model, firmware, operating mode, and software calibration;
- apply the LeRobot/LeLamp servo profile only where the live value differs;
- convert calibrated radians to and from circular 12-bit encoder values;
- exchange synchronized position commands and state feedback; and
- tie torque enable/disable to the ROS 2 hardware lifecycle.

The physical launch entry point is:

```bash
cd /home/mofe/Desktop/dev/orion/ros2_ws
source /opt/ros/jazzy/setup.bash
source install/setup.bash
ros2 launch orion_description hardware.launch.py
```

Launching the controller manager configures the servos and activates holding
torque at their measured positions. It does not send a named pose. The default
connection is `/dev/ttyACM0` at 1 Mbaud and the default calibration is
`~/.config/orion/servo_calibration.json`; all three are launch arguments.

## Servo profile

Persistent writes are performed torque-off, only if a value differs, and are
read back before the EEPROM is relocked:

- return delay `0`;
- position mode `0`;
- phase bit `0x10` cleared;
- PID `P=16`, `I=0`, `D=32`; and
- maximum acceleration `254`.

Runtime acceleration is set to `254`. The driver deliberately does not write
`Goal_Velocity`, `Torque_Limit`, `Max_Torque_Limit`, protection current, or
temperature limits. Position commands use a two-byte sync write beginning at
`Goal_Position`, so those neighboring registers are not changed as a side
effect.

## Vendor SDK

The package vendors the MIT-licensed official
[`ftservo/FTServo_Linux`](https://github.com/ftservo/FTServo_Linux) SDK at
commit `06fd3356dbd7bccd886b5a70d7ae0fccc6c76d38`. Orion's local patch fixes the
serial descriptor close order, closes the descriptor when serial setup fails,
uses `delete[]` for the SDK sync-read buffer, and removes an unconditional
baud-rate print. The upstream license is preserved in `vendor/ftservo/LICENSE`.

## Verification

```bash
cd /home/mofe/Desktop/dev/orion/ros2_ws
source /opt/ros/jazzy/setup.bash
colcon build --packages-select orion_hardware orion_description
source install/setup.bash
colcon test --packages-select orion_hardware orion_description
colcon test-result --verbose
```
