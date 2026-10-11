#!/usr/bin/env bash
# Source-built, headless camera stack for Ubuntu 24.04 on Raspberry Pi 5.
set -euo pipefail

if [[ ${1:-} == --help ]]; then
    echo "Usage: sudo $0 [video-group-user]"
    exit 0
fi
if (( $# > 1 )); then
    echo "Usage: sudo $0 [video-group-user]" >&2
    exit 2
fi
if (( EUID != 0 )); then
    echo "Run this installer with sudo." >&2
    exit 1
fi
# Do not purge packages on an unrelated host.
source /etc/os-release
if [[ ${ID:-} != ubuntu || ${VERSION_ID:-} != 24.04 || $(uname -m) != aarch64 ]] \
    || ! grep -q 'Raspberry Pi 5' /proc/device-tree/model; then
    echo "Requires Ubuntu 24.04 aarch64 on Raspberry Pi 5." >&2
    exit 1
fi
camera_user=${1:-${SUDO_USER:-}}
if [[ -n ${camera_user} ]]; then
    id "${camera_user}" >/dev/null
fi

# Pin tags AND commits: fail if an upstream tag is moved.
libpisp_tag=v1.2.0
libpisp_commit=50426319aa1a9ba4672f91977429365ad4e335a2
libcamera_tag=v0.5.0+rpt20250429
libcamera_commit=d83ff0a4ae4503bc56b7ed48cd142c3dd423ad3b
rpicam_tag=v1.7.0
rpicam_commit=5a3f5965aca96c2d575261e4e4045f0e0481279b
kmsxx_commit=b91affd0725f2523e2563da4f6295a2977fd3945

apt-get update
# Enumerate installed names rather than passing apt a broad wildcard. Do not
# autoremove: other Orion components may depend on orphaned packages.
conflicts=()
while IFS= read -r package; do
    conflicts+=("${package}")
done < <(
    dpkg-query -W -f='${binary:Package}\t${db:Status-Abbrev}\n' \
        'libcamera*' 'rpicam-apps*' 'python3-libcamera' 2>/dev/null \
        | awk '$2 ~ /^ii/ { print $1 }'
)
if (( ${#conflicts[@]} )); then
    apt-get purge -y "${conflicts[@]}"
fi
apt-get install -y --no-install-recommends \
    build-essential git meson ninja-build pkg-config cmake curl \
    libboost-dev libboost-program-options-dev nlohmann-json3-dev \
    libgnutls28-dev libssl-dev libevent-dev libudev-dev libyaml-dev \
    libjpeg-dev libtiff-dev libpng-dev libexif-dev libdrm-dev libfmt-dev libcap-dev \
    python3-dev python3-jinja2 python3-yaml python3-ply python3-pybind11 pybind11-dev

build_root=$(mktemp -d /var/tmp/orion-camera.XXXXXXXX)
trap 'rm -rf -- "${build_root}"' EXIT
export PKG_CONFIG_PATH=/usr/local/lib/pkgconfig${PKG_CONFIG_PATH:+:${PKG_CONFIG_PATH}}
# Prefer the Ubuntu Python/compiler tools, even under a customised sudo PATH.
export PATH=/usr/bin:/bin:/usr/sbin:/sbin:/usr/local/bin:/usr/local/sbin

build_release() {
    local project=$1 tag=$2 expected=$3
    shift 3
    local source_root=${build_root}/${project}
    git clone --depth 1 --branch "${tag}" "https://github.com/raspberrypi/${project}.git" "${source_root}"
    if [[ $(git -C "${source_root}" rev-parse HEAD) != "${expected}" ]]; then
        echo "Pinned commit mismatch for ${project} ${tag}; refusing to build." >&2
        exit 1
    fi
    meson setup "${source_root}/build" "${source_root}" \
        --prefix=/usr/local --libdir=lib --buildtype=release --wrap-mode=nodownload "$@"
    # Two build jobs leave room for other Pi processes and limit build memory.
    meson compile -C "${source_root}/build" -j 2
    meson install -C "${source_root}/build"
    ldconfig
}

build_release libpisp "${libpisp_tag}" "${libpisp_commit}" -Dlogging=disabled -Dexamples=false
build_release libcamera "${libcamera_tag}" "${libcamera_commit}" \
    -Dpipelines=rpi/pisp -Dipas=rpi/pisp -Dpycamera=enabled \
    -Dcam=disabled -Dqcam=disabled -Dgstreamer=disabled \
    -Ddocumentation=disabled -Dtest=false -Dlc-compliance=disabled -Dtracing=disabled
build_release rpicam-apps "${rpicam_tag}" "${rpicam_commit}" \
    -Denable_libav=disabled -Denable_drm=disabled -Denable_egl=disabled \
    -Denable_qt=disabled -Denable_opencv=disabled -Denable_tflite=disabled \
    -Denable_hailo=disabled -Ddownload_hailo_models=false \
    -Denable_imx500=false -Ddownload_imx500_models=false

# Picamera2 imports pykms even with preview disabled. Ubuntu has no kms++
# Python package. Its Python __init__ also imports utils, so build both.
kmsxx_source=${build_root}/kmsxx
git init "${kmsxx_source}"
git -C "${kmsxx_source}" fetch --depth 1 https://github.com/tomba/kmsxx.git "${kmsxx_commit}"
git -C "${kmsxx_source}" checkout --detach FETCH_HEAD
test "$(git -C "${kmsxx_source}" rev-parse HEAD)" = "${kmsxx_commit}"
meson setup "${kmsxx_source}/build" "${kmsxx_source}" \
    --prefix=/usr/local --libdir=lib --buildtype=release --wrap-mode=nodownload \
    -Dpykms=enabled -Dlibutils=true -Dutils=true -Domap=disabled -Dkmscube=false
meson compile -C "${kmsxx_source}/build" -j 2
meson install -C "${kmsxx_source}/build"
ldconfig

# libcamera uses site-packages, while Ubuntu's Python searches dist-packages.
# A .pth file bridges that path without replacing the distro Python packages.
python_version=$(/usr/bin/python3 -c 'import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")')
binding_root=/usr/local/lib/python${python_version}/site-packages
test -r "${binding_root}/libcamera/__init__.py"
install -d "/usr/local/lib/python${python_version}/dist-packages"
printf '%s\n' "${binding_root}" > "/usr/local/lib/python${python_version}/dist-packages/orion-libcamera.pth"
/usr/bin/python3 -c 'import libcamera; print("libcamera bindings:", libcamera.__file__)'
/usr/bin/python3 -c 'import pykms; print("pykms bindings:", pykms.__file__)'

getent group video >/dev/null || groupadd --system video
install -d /etc/udev/rules.d
printf '%s\n' 'SUBSYSTEM=="dma_heap", GROUP="video", MODE="0660"' \
    > /etc/udev/rules.d/70-orion-camera.rules
udevadm control --reload-rules
udevadm trigger --subsystem-match=dma_heap
if [[ -n ${camera_user} ]]; then
    usermod -a -G video "${camera_user}"
fi

# The inference asset is data, not a captured frame. Pin the upstream commit
# and verify before replacing the installed model, including on repeated runs.
yunet_file=face_detection_yunet_2023mar.onnx
yunet_sha=8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4
yunet_url=https://media.githubusercontent.com/media/opencv/opencv_zoo/47534e27c9851bb1128ccc0102f1145e27f23f98/models/face_detection_yunet/${yunet_file}
curl --fail --location --retry 3 "${yunet_url}" --output "${build_root}/${yunet_file}"
printf '%s  %s\n' "${yunet_sha}" "${build_root}/${yunet_file}" | sha256sum --check --status
install -d /usr/local/share/orion/vision
install -m 0644 "${build_root}/${yunet_file}" "/usr/local/share/orion/vision/${yunet_file}"

echo "Installed libpisp ${libpisp_tag}, libcamera ${libcamera_tag}, rpicam-apps ${rpicam_tag}."
echo "Run vision/scripts/verify-camera-stack.sh. If the sensor is missing, configure its IMX708 overlay for the fitted CSI port and reboot."
echo "Use a fresh login after changing video-group membership."
