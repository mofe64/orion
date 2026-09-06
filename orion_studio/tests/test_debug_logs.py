import sys
import unittest
from pathlib import Path
from unittest.mock import patch, Mock
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from gateway import OrionGateway, GatewayError
class DebugLogsTest(unittest.TestCase):
    def test_only_fixed_services_and_bounded_recent_lines(self):
        with patch('gateway.shutil.which', return_value='/usr/bin/journalctl'), patch('gateway.subprocess.run', return_value=Mock(returncode=0, stdout='entry\n'*300)) as run:
            result = OrionGateway.runtime_logs(None)
            self.assertEqual(len(result['lines']), 200)
            args, kwargs = run.call_args
            self.assertEqual(args[0][-6:], ['-u','oriond.service','-u','orion-studio-gateway.service','-u','orion-listener.service'])
            self.assertEqual(kwargs['timeout'], 3)
            self.assertNotIn('shell', kwargs)
    def test_unavailable_and_denied_are_explicit(self):
        with patch('gateway.shutil.which', return_value=None):
            with self.assertRaises(GatewayError): OrionGateway.runtime_logs(None)
        with patch('gateway.shutil.which', return_value='journalctl'), patch('gateway.subprocess.run', return_value=Mock(returncode=1, stdout='')):
            with self.assertRaises(GatewayError): OrionGateway.runtime_logs(None)
