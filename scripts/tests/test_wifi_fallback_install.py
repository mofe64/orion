"""Install failures restore files; repeat installs preserve hotspot credentials."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import install_pi_wifi_fallback as installer

HOME = 'e2dee799-7aff-3212-b6ba-72fb7cacaf1f'
HOTSPOT = 'bbbbbbbb-1234-4321-8765-123456789abc'


class InstallTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        for name, value in {'CONFIG': root / 'config.env', 'HELPER': root / 'helper',
                            'UNITS': root / 'units', 'BACKUPS': root / 'backups'}.items():
            patcher = patch.object(installer, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        self.calls = []
        self.existing = False
        self.fail_enable = False
        self.fail_create = False
        self.state = '100 (connected)'
        self.country = 'GB'
        self.fields = {'802-11-wireless.mode': 'ap', '802-11-wireless.ssid': 'Orion-Ariadne',
                       'connection.interface-name': 'wlan0', 'connection.autoconnect': 'no',
                       'ipv4.method': 'shared', 'ipv4.addresses': '10.42.0.1/24',
                       'ipv6.method': 'ignore', '802-11-wireless-security.key-mgmt': 'wpa-psk'}
        patcher = patch.object(installer, 'run', self.command)
        patcher.start()
        self.addCleanup(patcher.stop)

    def command(self, *arguments, check=True):
        self.calls.append(arguments)
        output, code = '', 0
        if arguments[:3] == ('iw', 'reg', 'get'):
            output = f'global\ncountry {self.country}: DFS-ETSI\n'
        elif arguments[0] == 'dpkg-query':
            output = 'install ok installed'
        elif arguments[0] == 'ip':
            output = json.dumps([{'ifname': 'wlan0', 'addr_info': [{'local': '192.168.1.191', 'prefixlen': 24}]}])
        elif arguments[:2] == ('nmcli', '-g'):
            field = arguments[2]
            if arguments[3:5] == ('device', 'show'):
                output = {'WIFI-PROPERTIES.AP': 'yes', 'GENERAL.STATE': self.state,
                          'GENERAL.CON-UUID': HOME}[field]
            elif arguments[5] == 'id':
                output, code = (HOTSPOT, 0) if self.existing else ('', 10)
            elif arguments[-1] == HOME:
                output = {'802-11-wireless.mode': 'infrastructure', 'connection.autoconnect': 'yes',
                          'connection.id': 'Home Wi-Fi'}[field]
            else:
                output = self.fields[field]
        elif arguments[:2] == ('systemctl', 'is-enabled') or arguments[:3] == ('systemctl', 'is-active', '--quiet'):
            code = 0 if arguments[-1] == 'NetworkManager' else 1
        elif arguments[:3] == ('systemctl', 'enable', '--now') and self.fail_enable:
            raise RuntimeError('Injected timer activation failure')
        elif arguments[:3] == ('nmcli', 'connection', 'add') and self.fail_create:
            raise subprocess.TimeoutExpired('nmcli', 30)
        return subprocess.CompletedProcess(arguments, code, output, '')

    def plan(self):
        return installer.preflight('wlan0', 'Orion-Ariadne', 'GB')

    def test_plan_has_no_filesystem_or_network_mutations(self):
        self.plan()
        self.assertFalse(installer.CONFIG.exists())
        self.assertFalse(installer.BACKUPS.exists())
        self.assertFalse(any(command[:3] == ('nmcli', 'connection', 'add') for command in self.calls))
        self.assertFalse(any(command[:2] == ('systemctl', 'enable') for command in self.calls))

    def test_existing_foreign_profile_is_rejected(self):
        self.existing = True
        with self.assertRaisesRegex(RuntimeError, 'refusing to overwrite'):
            self.plan()
        self.assertFalse(installer.CONFIG.exists())

    def own_existing_profile(self):
        self.existing = True
        installer.CONFIG.write_text(f'ORION_WIFI_INTERFACE=wlan0\nORION_WIFI_HOME_UUID={HOME}\nORION_WIFI_HOTSPOT_UUID={HOTSPOT}\n')

    def test_repeat_install_preserves_credentials(self):
        self.own_existing_profile()
        home, hotspot, password, _ = self.plan()
        self.assertEqual((home, hotspot, password), (HOME, HOTSPOT, None))
        self.assertFalse(any('wifi-sec.psk' in command for command in self.calls))

    def test_modified_existing_hotspot_is_not_overwritten(self):
        self.own_existing_profile()
        self.fields['connection.autoconnect'] = 'yes'
        with self.assertRaisesRegex(RuntimeError, 'unexpected connection.autoconnect'):
            self.plan()

    def test_install_rejects_a_busy_device(self):
        self.state = '70 (connecting)'
        with self.assertRaisesRegex(RuntimeError, 'normal Wi-Fi is connected'):
            self.plan()

    def test_install_requires_the_expected_regulatory_country(self):
        self.country = '00'
        with self.assertRaisesRegex(RuntimeError, 'regulatory country'):
            self.plan()

    def test_timer_failure_restores_files_and_removes_inactive_new_profile(self):
        home, hotspot, password, files = self.plan()
        self.fail_enable = True
        with self.assertRaisesRegex(RuntimeError, 'Injected timer'):
            installer.install('wlan0', 'Orion-Ariadne', home, hotspot, password, files)
        for path in [installer.CONFIG, *files]:
            self.assertFalse(path.exists(), str(path))
        self.assertIn(('nmcli', 'connection', 'delete', 'uuid', hotspot), self.calls)
        create = next(command for command in self.calls if command[:3] == ('nmcli', 'connection', 'add'))
        self.assertEqual(create[create.index('connection.autoconnect') + 1], 'no')
        self.assertIn('wifi-sec.psk', create)
        self.assertFalse(any(command[:3] == ('nmcli', 'connection', 'up') for command in self.calls))

    def test_failed_update_restores_the_previous_helper_and_config(self):
        self.own_existing_profile()
        installer.HELPER.write_text('previous helper')
        installer.HELPER.chmod(0o755)
        config = installer.CONFIG.read_bytes()
        home, hotspot, password, files = self.plan()
        self.fail_enable = True
        with self.assertRaisesRegex(RuntimeError, 'Injected timer'):
            installer.install('wlan0', 'Orion-Ariadne', home, hotspot, password, files)
        self.assertEqual(installer.CONFIG.read_bytes(), config)
        self.assertEqual(installer.HELPER.read_text(), 'previous helper')
        self.assertFalse(any(command[:3] == ('nmcli', 'connection', 'delete') for command in self.calls))

    def test_unacknowledged_profile_creation_is_cleaned_up_by_reserved_uuid(self):
        home, hotspot, password, files = self.plan()
        self.fail_create = True
        with self.assertRaises(subprocess.TimeoutExpired):
            installer.install('wlan0', 'Orion-Ariadne', home, hotspot, password, files)
        self.assertIn(('nmcli', 'connection', 'delete', 'uuid', hotspot), self.calls)
        self.assertFalse(installer.CONFIG.exists())


if __name__ == '__main__':
    unittest.main()
