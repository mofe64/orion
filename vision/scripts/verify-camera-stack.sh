#!/usr/bin/env bash
# Read-only camera enumeration through the release's Python 3.12 venv.
set -euo pipefail
release=${1:?Usage: verify-camera-stack.sh /path/to/installed/release}
python=$release/vision/.venv/bin/python
[[ -x $python ]] || { echo "Missing vision interpreter: $python" >&2; exit 1; }
"$python" - <<'PY'
import libcamera, pykms
from picamera2 import Picamera2
cameras = Picamera2.global_camera_info()
print(cameras)
if not any('imx708' in str(camera).lower() for camera in cameras):
    raise SystemExit('IMX708 not detected; check the ribbon, CSI port and camera overlay.')
print('PASS: Python 3.12 camera bindings and IMX708 enumeration.')
PY
