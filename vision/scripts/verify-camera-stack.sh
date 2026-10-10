#!/usr/bin/env bash
# Read-only commissioning check; run as the user who will own the camera.
set -euo pipefail

hello=/usr/local/bin/rpicam-hello
if [[ ! -x ${hello} ]]; then
    echo "FAIL: missing ${hello}; run install-camera-stack.sh." >&2
    exit 1
fi
if ! cameras=$(timeout 20s "${hello}" --list-cameras 2>&1); then
    printf 'FAIL: camera enumeration failed:\n%s\n' "${cameras}" >&2
    exit 1
fi
printf '%s\n' "${cameras}"
if ! grep -q 'imx708_wide' <<< "${cameras}"; then
    echo "FAIL: imx708_wide not detected; check the ribbon, CSI port and IMX708 boot overlay." >&2
    exit 1
fi
if ! python3 -c 'import libcamera; print("libcamera bindings:", libcamera.__file__)'; then
    echo "FAIL: Python cannot import libcamera; check the system Python and binding .pth file." >&2
    exit 1
fi
for module in rp1_cfe pisp_be imx708; do
    # sysfs also recognises drivers built into the kernel rather than modules
    # listed by lsmod, which is important across Ubuntu kernel updates.
    if [[ ! -d /sys/module/${module} ]]; then
        echo "FAIL: ${module} is not loaded; check linux-raspi and the camera overlay." >&2
        exit 1
    fi
done
echo "PASS: imx708_wide, Python bindings and Pi 5 camera drivers detected."
