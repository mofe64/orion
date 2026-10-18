import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import release_build


class NativeBindingTests(unittest.TestCase):
    def wheel(self, folder, module):
        path = Path(folder) / 'rpi_libcamera-0.1a11-cp312-cp312-linux_aarch64.whl'
        with zipfile.ZipFile(path, 'w') as archive:
            archive.writestr('libcamera/_libcamera.so', module)
        return path

    def test_camera_binding_must_use_the_pinned_pybind11(self):
        with tempfile.TemporaryDirectory() as folder:
            old = self.wheel(folder, b'\0__pybind11_internals_v5_gcc_libstdcpp_cxxabi1018__\0')
            release_build.check_pybind11(old, '2.13.6')
            with self.assertRaisesRegex(RuntimeError, 'not built with pybind11 3.0.1'):
                release_build.check_pybind11(old, '3.0.1')
            new = self.wheel(folder, b'\0__pybind11_internals_v12_gcc_libstdcpp_cxxabi1018__\0')
            with self.assertRaisesRegex(RuntimeError, 'not built with pybind11 2.13.6'):
                release_build.check_pybind11(new, '2.13.6')

    def test_build_constraints_reach_isolated_builds_on_old_and_new_pip(self):
        seen = {}
        def run(*args, env=None, **kwargs):
            seen.update(env)
            seen['text'] = Path(env['PIP_BUILD_CONSTRAINT']).read_text()
        with tempfile.TemporaryDirectory() as folder, patch.object(release_build, 'run', side_effect=run):
            release_build.wheel('source', destination=Path(folder) / 'out', constraints=['pybind11==2.13.6'])
        self.assertEqual(seen['PIP_CONSTRAINT'], seen['PIP_BUILD_CONSTRAINT'])
        self.assertEqual(seen['text'], 'pybind11==2.13.6\n')


if __name__ == '__main__':
    unittest.main()
