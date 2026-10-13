"""Install the deterministic speech peer into a temporary Python environment."""
from pathlib import Path
import shutil
import subprocess
import sys
import venv

environment = Path(sys.argv[1])
venv.EnvBuilder(with_pip=False).create(environment)
python = environment / 'bin/python'
packages = Path(subprocess.check_output(
    [str(python), '-I', '-c', 'import sysconfig; print(sysconfig.get_path("purelib"))'],
    text=True).strip())
source = Path(__file__).parent / 'orion_speech_worker'
destination = packages / 'orion_speech_worker'
shutil.copytree(source, destination)
(destination / '__init__.py').write_text('')
if len(sys.argv) > 2:
    worker = destination / 'worker.py'
    worker.write_text(worker.read_text().replace("model='fixture'", f'model={sys.argv[2]!r}'))
