"""Boot verification checks pin state without elevating the whole verifier."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


VERIFIER = Path(__file__).resolve().parents[2] / 'hardware/lighting/verify-persistent.sh'


class LightingVerifierTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.modules = self.root / 'modules'
        self.modules.write_text('rp1_ws281x_pwm 16384 0 - Live 0x0\n')
        self.channel = self.root / 'channel'
        self.channel.write_text('0\n')
        self.sudo_log = self.root / 'sudo-log'
        self.tool('pinctrl', '''
if [[ ${TEST_DENY_PIN_READ:-0} == 1 && ${TEST_ELEVATED:-0} != 1 ]]; then
    echo "Must be root (or group 'gpio' on RPiOS)"
    exit 255
fi
echo "${TEST_PIN_STATE:-12: a0 pn | lo // GPIO12 = PWM0_CHAN0}"
''')
        self.tool('sudo', '''
printf '%s\\n' "$*" >> "$TEST_SUDO_LOG"
if [[ ${TEST_DENY_SUDO:-0} == 1 ]]; then
    echo 'sudo: authorization failed' >&2
    exit 1
fi
export TEST_ELEVATED=1
exec "$@"
''')
        self.tool('systemctl', 'exit 0\n')
        # Supply fake kernel state and tools; /dev/null is a real accessible char device.
        source = VERIFIER.read_text().replace('/dev/ws281x_pwm', '/dev/null')
        source = source.replace('/proc/modules', str(self.modules))
        source = source.replace('/sys/module/rp1_ws281x_pwm/parameters/pwm_channel', str(self.channel))
        source = source.replace('/usr/bin/pinctrl', str(self.root / 'pinctrl'))
        self.script = self.root / 'verify.sh'
        self.script.write_text(source)

    def tool(self, name, source):
        path = self.root / name
        path.write_text('#!/bin/bash\nset -euo pipefail\n' + source)
        path.chmod(0o755)

    def run_verifier(self, **settings):
        environment = dict(os.environ, PATH=f'{self.root}:{os.environ["PATH"]}',
                           TEST_SUDO_LOG=str(self.sudo_log), **settings)
        return subprocess.run(['bash', str(self.script)], env=environment,
                              capture_output=True, text=True)

    def test_unprivileged_gpio_read_does_not_use_sudo(self):
        result = self.run_verifier()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('PASS:', result.stdout)
        self.assertFalse(self.sudo_log.exists())

    def test_ubuntu_root_only_gpio_read_retries_only_pinctrl(self):
        result = self.run_verifier(TEST_DENY_PIN_READ='1')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('PASS:', result.stdout)
        self.assertEqual(self.sudo_log.read_text(), f'{self.root}/pinctrl get 12\n')

    def test_failed_elevated_read_reports_the_error_without_pass(self):
        result = self.run_verifier(TEST_DENY_PIN_READ='1', TEST_DENY_SUDO='1')
        self.assertEqual(result.returncode, 1)
        self.assertIn('FAIL: could not read GPIO12 state:', result.stderr)
        self.assertIn('sudo: authorization failed', result.stderr)
        self.assertNotIn('PASS:', result.stdout)

    def test_wrong_pin_function_is_rejected_after_elevated_read(self):
        result = self.run_verifier(TEST_DENY_PIN_READ='1', TEST_PIN_STATE='12: ip pn // GPIO12 = input')
        self.assertEqual(result.returncode, 1)
        self.assertIn('FAIL: BCM12 is not configured', result.stderr)
        self.assertNotIn('PASS:', result.stdout)

    def test_driver_failure_stops_before_any_elevated_read(self):
        self.modules.write_text('')
        result = self.run_verifier(TEST_DENY_PIN_READ='1')
        self.assertEqual(result.returncode, 1)
        self.assertIn('rp1_ws281x_pwm is not loaded', result.stderr)
        self.assertFalse(self.sudo_log.exists())
