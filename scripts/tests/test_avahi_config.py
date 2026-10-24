from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from avahi_config import configure_avahi, deny_interfaces


class AvahiTests(unittest.TestCase):
    def test_merge_preserves_sections_comments_and_operator_interfaces(self):
        text = '[server]\nhost-name=ariadne\ndeny-interfaces=virbr0 # local\n[publish]\npublish-workstation=yes\n'
        updated = deny_interfaces(text, ['docker0', 'hassio'])
        self.assertIn('deny-interfaces=virbr0,docker0,hassio # local', updated)
        self.assertIn('host-name=ariadne', updated)
        self.assertIn('[publish]\npublish-workstation=yes', updated)
        self.assertEqual(deny_interfaces(updated, ['docker0', 'hassio']), updated)
        self.assertIn('[server]\ndeny-interfaces=docker0\n', deny_interfaces('[publish]\n', ['docker0']))
        self.assertEqual(deny_interfaces('[server]', ['docker0']), '[server]\ndeny-interfaces=docker0\n')

    def test_existing_lamp_update_excludes_hassio_only_when_present_and_restarts(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            path = root / 'avahi.conf'
            path.write_text('[server]\nuse-ipv6=yes\n')
            (root / 'hassio').mkdir()
            run = Mock()
            self.assertTrue(configure_avahi(run, prefix=('sudo',), path=path, net=root))
            self.assertIn('deny-interfaces=docker0,hassio', run.call_args_list[0].kwargs['input'])
            self.assertEqual(run.call_args.args, ('sudo', 'systemctl', 'restart', 'avahi-daemon'))
