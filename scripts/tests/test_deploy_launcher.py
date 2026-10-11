"""Exercise probe/build/upload ordering and SSH cleanup without operating hardware."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]


class DeploymentLauncherTests(unittest.TestCase):
    def exercise(self, fail_scp=0, fail_remote=False, fail_build=False, wrong_platform=False):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            binaries = directory / 'bin'; binaries.mkdir()
            log = directory / 'calls'
            fake = '''import json, os, pathlib, shutil, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ['ORION_LAUNCH_LOG'], 'a') as log:
    log.write(json.dumps([name, *args])+'\\n')
if name == 'git':
    if 'rev-parse' in args: print('a'*40)
    sys.exit(0)
if name == 'python3':
    if '--snapshot-only' in args:
        release = pathlib.Path(args[args.index('--output')+1]) / ('a'*12+'-12345678')
        shutil.copytree(os.environ['ORION_SCRIPTS'], release / 'scripts')
        script = release / 'scripts/pi_deploy_remote.sh'
        script.write_text(script.read_text()+'\\n# selected committed helper\\n')
        pathlib.Path(os.environ['ORION_COMMITTED_SCRIPT']).write_bytes(script.read_bytes())
        print(release)
        sys.exit(0)
    if '--prepared-source' in args:
        if os.environ['ORION_FAIL_BUILD'] == '1': sys.exit(31)
        pathlib.Path(args[args.index('--output')+1]).write_bytes(b'prepared release')
        sys.exit(0)
    os.execv(os.environ['ORION_REAL_PYTHON'], [os.environ['ORION_REAL_PYTHON'], *args])
if name == 'ssh':
    if '--probe' in args[-1]:
        target = {'id':'ubuntu','version_id':'24.04','machine':'aarch64','system_python':'3.12','libc':'glibc-2.39'}
        if os.environ['ORION_WRONG_PLATFORM'] == '1': target['version_id'] = '26.04'
        print(json.dumps(target))
    elif 'mktemp' in args: print('/tmp/orion-deploy.fixture')
    elif '-t' in args:
        if sys.stdin.read(): sys.exit('Script consumed the password input channel')
        sys.exit(23 if os.environ['ORION_FAIL_REMOTE'] == '1' else 0)
if name == 'scp':
    count_file = pathlib.Path(os.environ['ORION_SCP_COUNT'])
    count = int(count_file.read_text())+1 if count_file.exists() else 1
    count_file.write_text(str(count))
    if count == int(os.environ['ORION_FAIL_SCP']): sys.exit(17)
    if args[-1].endswith('/install.sh'):
        pathlib.Path(os.environ['ORION_TRANSFER']).write_bytes(pathlib.Path(args[-2]).read_bytes())
'''
            for name in ('ssh', 'scp', 'python3', 'git', 'docker'):
                path = binaries / name
                path.write_text(f'#!{sys.executable}\n' + fake); path.chmod(0o755)
            env = {**os.environ, 'PATH': str(binaries) + ':' + os.environ['PATH'],
                   'ORION_LAUNCH_LOG': str(log), 'ORION_TRANSFER': str(directory / 'uploaded-script'),
                   'ORION_COMMITTED_SCRIPT': str(directory / 'committed-script'),
                   'ORION_REAL_PYTHON': sys.executable, 'ORION_SCRIPTS': str(SCRIPTS),
                   'ORION_SCP_COUNT': str(directory / 'count'), 'ORION_FAIL_SCP': str(fail_scp),
                   'ORION_FAIL_REMOTE': str(int(fail_remote)), 'ORION_FAIL_BUILD': str(int(fail_build)),
                   'ORION_WRONG_PLATFORM': str(int(wrong_platform))}
            result = subprocess.run(['bash', str(SCRIPTS / 'deploy_pi.sh'), '--hardware', 'v1', '--skip-studio-check',
                                     '--host', 'pi@robot.local', '--root', '/home/pi/orion', '--branch', 'main'],
                                    env=env, input='', text=True, capture_output=True)
            expected = 17 if fail_scp else 23 if fail_remote else 31 if fail_build else 1 if wrong_platform else 0
            self.assertEqual(result.returncode, expected, result.stderr)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            ssh_calls = [call for call in calls if call[0] in ('ssh', 'scp')]
            sockets = [next(arg for arg in call if arg.startswith('ControlPath=')) for call in ssh_calls]
            self.assertEqual(len(set(sockets)), 1)
            local_workspace = Path(sockets[0].split('=', 1)[1]).parent.parent
            self.assertFalse(local_workspace.exists())
            self.assertTrue(any('-O' in call and 'exit' in call for call in ssh_calls))
            launch = [call for call in calls if call[0] == 'ssh' and '-t' in call]
            builds = [call for call in calls if call[0] == 'python3' and '--prepared-source' in call]
            transfers = [call for call in calls if call[0] == 'scp']
            probe = next(call for call in calls if call[0] == 'ssh' and '--probe' in call[-1])
            if wrong_platform:
                self.assertEqual(builds, []); self.assertEqual(transfers, [])
            else:
                self.assertLess(calls.index(probe), calls.index(builds[0]))
                self.assertIn('--skip-studio-check', builds[0])
            if fail_build or wrong_platform:
                self.assertEqual(transfers, [])
            if fail_scp or fail_build or wrong_platform:
                self.assertEqual(launch, [])
            else:
                self.assertEqual((directory / 'uploaded-script').read_bytes(), (directory / 'committed-script').read_bytes())
                self.assertEqual(len(launch), 1)
                self.assertIn("'/home/pi/orion' '/tmp/orion-deploy.fixture/release.tar.gz'", launch[0][-1])
                self.assertIn("trap 'rm -rf -- /tmp/orion-deploy.fixture' EXIT", launch[0][-1])
                self.assertLess(calls.index(builds[0]), calls.index(transfers[0]))
            if transfers:
                cleanup = next(call for call in calls if call[-4:] == ['rm', '-rf', '--', '/tmp/orion-deploy.fixture'])
                self.assertIn('BatchMode=yes', cleanup)

    def test_probes_builds_and_uploads_with_terminal_input_available(self): self.exercise()
    def test_script_transfer_failure_cleans_up_without_installation(self): self.exercise(fail_scp=1)
    def test_bundle_transfer_failure_cleans_up_without_installation(self): self.exercise(fail_scp=2)
    def test_remote_failure_is_propagated(self): self.exercise(fail_remote=True)
    def test_build_failure_does_not_upload_or_install(self): self.exercise(fail_build=True)
    def test_wrong_robot_platform_stops_before_build_or_transfer(self): self.exercise(wrong_platform=True)


if __name__ == '__main__':
    unittest.main()
