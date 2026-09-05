import unittest

from orion_servo_setup.preflight import PreflightError, commissioning_plan, read_preflight
from orion_servo_setup.provisioning import ORION_SERVO_ASSIGNMENTS


class ReadOnlyBus:
    """No write or torque methods: preflight must work without physical authority."""

    def __init__(self):
        self.model = 777
        self.values = {
            "Operating_Mode": 0, "Torque_Enable": 0, "Present_Position": 2048,
            "Present_Voltage": 62, "Present_Temperature": 25, "Status": 0,
        }

    def ping(self, motor, **kwargs):
        return self.model

    def read(self, data_name, motor, **kwargs):
        return self.values[data_name]


class PreflightTests(unittest.TestCase):
    def test_reads_all_joints_without_write_capability(self):
        plan = commissioning_plan(reversed(ORION_SERVO_ASSIGNMENTS))
        self.assertEqual(plan, ORION_SERVO_ASSIGNMENTS)
        snapshots = read_preflight(ReadOnlyBus(), plan)
        self.assertEqual(tuple(item.assignment for item in snapshots), plan)
        for item in snapshots:
            self.assertEqual((item.position_raw, item.voltage_v, item.temperature_c),
                             (2048, 6.2, 25))

    def test_rejects_unsafe_bus_states(self):
        for register, value, message in (
            ("Operating_Mode", 1, "position mode"),
            ("Torque_Enable", 1, "already has torque enabled"),
            ("Present_Position", -1, "invalid raw position"),
            ("Present_Position", 4096, "invalid raw position"),
            ("Present_Voltage", 54, "5.5-6.6 V"),
            ("Present_Voltage", 67, "5.5-6.6 V"),
            ("Present_Temperature", 41, "allow it to cool"),
            ("Status", 1, "servo status"),
        ):
            with self.subTest(register=register, value=value):
                bus = ReadOnlyBus()
                bus.values[register] = value
                with self.assertRaisesRegex(PreflightError, message):
                    read_preflight(bus, ORION_SERVO_ASSIGNMENTS)

    def test_rejects_wrong_servo_model(self):
        bus = ReadOnlyBus()
        bus.model = 123
        with self.assertRaisesRegex(PreflightError, "expected STS3215"):
            read_preflight(bus, ORION_SERVO_ASSIGNMENTS)

    def test_accepts_existing_limits_at_boundaries(self):
        for position, voltage in ((0, 55), (4095, 66)):
            bus = ReadOnlyBus()
            bus.values.update(Present_Position=position, Present_Voltage=voltage,
                              Present_Temperature=40)
            self.assertEqual(len(read_preflight(bus, ORION_SERVO_ASSIGNMENTS)), 5)
