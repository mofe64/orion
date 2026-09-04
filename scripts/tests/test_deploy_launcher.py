"""Validate SSH terminal setup without opening a network connection or running the robot."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]


class DeploymentLauncherTests(unittest.TestCase):
    def exercise(self, fail_scp=False, fail_remote=False):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            binaries = directory / 'bin'; binaries.mkdir()
            log = directory / 'calls'
            transferred = directory / 'uploaded-script'
            fake = '''#!/usr/bin/env python3
import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
with open(os.environ['ORION_LAUNCH_LOG'], 'a') as log: log.write(json.dumps([name, *sys.argv[1:]])+'\\n')
if name == 'ssh' and 'mktemp' in sys.argv: print('/tmp/orion-deploy.fixture')
if name == 'scp':
 if os.environ['ORION_FAIL_SCP'] == '1': sys.exit(17)
 pathlib.Path(os.environ['ORION_TRANSFER']).write_bytes(pathlib.Path(sys.argv[-2]).read_bytes())
if name == 'ssh' and '-t' in sys.argv:
 if sys.stdin.read(): sys.exit('Script consumed the password input channel')
 sys.exit(23 if os.environ['ORION_FAIL_REMOTE'] == '1' else 0)
'''
            for name in ['ssh', 'scp']:
                path = binaries / name; path.write_text(fake); path.chmod(0o755)
            env = {**os.environ, 'PATH': str(binaries) + ':' + os.environ['PATH'],
                   'ORION_LAUNCH_LOG': str(log), 'ORION_TRANSFER': str(transferred),
                   'ORION_FAIL_SCP': str(int(fail_scp)), 'ORION_FAIL_REMOTE': str(int(fail_remote))}
            result = subprocess.run(['bash', str(SCRIPTS / 'deploy_pi.sh'), '--skip-studio-check',
                                     '--host', 'pi@robot.local', '--root', '/home/pi/orion', '--branch', 'main'],
                                    env=env, input='', text=True, capture_output=True)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            self.assertEqual(result.returncode, 17 if fail_scp else 23 if fail_remote else 0, result.stderr)
            launch = [call for call in calls if call[0] == 'ssh' and '-t' in call]
            if fail_scp:
                self.assertEqual(launch, [])
            else:
                self.assertEqual(transferred.read_bytes(), (SCRIPTS / 'pi_deploy_remote.sh').read_bytes())
                self.assertEqual(len(launch), 1)
                self.assertIn("bash '/tmp/orion-deploy.fixture' '/home/pi/orion' 'main'", launch[0][-1])
                self.assertIn("trap 'rm -f -- /tmp/orion-deploy.fixture' EXIT", launch[0][-1])
            if fail_scp or fail_remote:
                cleanup = calls[-1]
                self.assertIn('BatchMode=yes', cleanup)
                self.assertEqual(cleanup[-4:], ['rm', '-f', '--', '/tmp/orion-deploy.fixture'])

    def test_uploads_script_and_reserves_terminal_input_for_authentication(self): self.exercise()
    def test_transfer_failure_cleans_up_without_starting_deployment(self): self.exercise(fail_scp=True)
    def test_remote_failure_is_propagated(self): self.exercise(fail_remote=True)


if __name__ == '__main__':
    unittest.main()
