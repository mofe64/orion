#!/usr/bin/env bash
set -euo pipefail

project_root="${1:?Pi project root is required}"
voice_home="${2:-${HOME}}"
if [[ ! "${project_root}" =~ ^/[A-Za-z0-9._/-]+$ || "${project_root}" == *".."* ]]; then
  echo "Refusing unsafe Pi project path" >&2
  exit 2
fi
if [[ "$(uname -s)" != Linux || "$(uname -m)" != aarch64 ]]; then
  echo "The Orion voice installer requires the commissioned 64-bit Raspberry Pi Linux host." >&2
  exit 1
fi
export PATH="${voice_home}/.cargo/bin:${voice_home}/.local/bin:${PATH}"
sudo -v
if [[ -L "${project_root}/voice" ]]; then
  echo "Refusing to replace a symlinked voice package." >&2
  exit 1
fi
# Install only missing application dependencies; leave the commissioned codec alone.
missing_packages=()
for package in alsa-utils build-essential pkg-config libssl-dev ca-certificates python3-venv; do
  if [[ "$(dpkg-query -W -f='${Status}' "${package}" 2>/dev/null)" != "install ok installed" ]]; then
    missing_packages+=("${package}")
  fi
done
if [[ ${#missing_packages[@]} -gt 0 ]]; then
  echo "Installing missing Pi voice packages: ${missing_packages[*]}"
  sudo apt-get update
  sudo apt-get install -y --no-install-recommends "${missing_packages[@]}"
else
  echo "Pi voice system packages are already installed."
fi
command -v cargo >/dev/null || { echo "The source-built Orion host requires Rust in ~/.cargo/bin." >&2; exit 1; }

# A dedicated bootstrap environment avoids system-pip/externally-managed conflicts.
uv_bootstrap="${voice_home}/.local/share/orion/uv-bootstrap"
if [[ ! -x "${uv_bootstrap}/bin/uv" ]]; then
  python3 -m venv "${uv_bootstrap}"
fi
if [[ "$("${uv_bootstrap}/bin/uv" --version 2>/dev/null || true)" != "uv 0.11.13" ]]; then
  "${uv_bootstrap}/bin/python" -m pip install --disable-pip-version-check 'uv==0.11.13'
fi
uv_command="${uv_bootstrap}/bin/uv"

if systemctl cat orion-listener.service >/dev/null 2>&1; then
  sudo systemctl stop orion-listener.service
fi
voice_environment="${project_root}/voice/.venv"
if [[ -L "${voice_environment}" ]]; then
  echo "Refusing to update a symlinked voice environment." >&2
  exit 1
fi
# Sync in place: uv reuses matching packages and removes dependencies absent
# from the lockfile. A failed update leaves the listener stopped for repair.
trap 'echo "Voice installation failed. Listener remains stopped; rerun install_pi_voice.sh after resolving the error." >&2' ERR
UV_PROJECT_ENVIRONMENT="${voice_environment}" "${uv_command}" sync --project "${project_root}/voice" --locked --python 3.12 --no-default-groups
"${voice_environment}/bin/python" -m unittest discover -s "${project_root}/voice/tests" -q
"${voice_environment}/bin/python" - "${project_root}" <<'PY'
from pathlib import Path
import sys
from orion_voice.rustpotter import RustpotterWakeDetector
root = Path(sys.argv[1])
wake = RustpotterWakeDetector(root / 'voice/models/wake/hey_orion_reference.rpw', .4)
wake.process(bytes(640))
print('Pi Rustpotter reference loaded and accepted a 20 ms silence frame.')
PY
trap - ERR
echo 'Rustpotter voice installed and verified.'
