#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/deploy_pi.sh [--host USER@HOST] [--root PATH] [--branch BRANCH] [--skip-studio-check]

Deploy and physically smoke-test Orion through SSH. Defaults:
  host:   mofe@orion.local
  root:   /home/mofe/dev/orion
  branch: main

The workstation must trust the Pi's SSH host key. SSH keys are recommended;
password login is also supported through one shared SSH connection. The remote script installs and enables oriond,
the Studio gateway and the Rustpotter listener during the supervised physical
smoke test. Deployment opens an SSH terminal so sudo can request the Pi user's
password. The account must be permitted to install packages and manage services.
It installs Pi voice dependencies and builds Rustpotter. Qwen and Chatterbox stay on Studio.

The matching Studio v2 frontend is tested and production-built locally before
SSH deployment. Use --skip-studio-check only if the exact revision was already
validated on another supported build host.
EOF
}

pi_host="${ORION_PI_HOST:-mofe@orion.local}"
project_root="${ORION_PI_ROOT:-/home/mofe/dev/orion}"
branch="${ORION_PI_BRANCH:-main}"
studio_check=true

while [[ $# -gt 0 ]]; do
  case "$1" in
    --host) pi_host="${2:?--host requires USER@HOST}"; shift 2 ;;
    --root) project_root="${2:?--root requires PATH}"; shift 2 ;;
    --branch) branch="${2:?--branch requires BRANCH}"; shift 2 ;;
    --skip-studio-check) studio_check=false; shift ;;
    --help|-h) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

if [[ ! "${project_root}" =~ ^/[A-Za-z0-9._/-]+$ || "${project_root}" == *".."* ]]; then
  echo "Refusing unsafe Pi project path: ${project_root}" >&2
  exit 2
fi
if [[ ! "${branch}" =~ ^[A-Za-z0-9._/-]+$ || "${branch}" == -* || "${branch}" == *".."* ]]; then
  echo "Refusing unsafe Git branch: ${branch}" >&2
  exit 2
fi
if [[ ! "${pi_host}" =~ ^[A-Za-z0-9._-]+@[A-Za-z0-9._-]+$ ]]; then
  echo "Refusing unsafe SSH target: ${pi_host}" >&2
  exit 2
fi

script_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_checkout="$(cd "${script_directory}/.." && pwd)"
if [[ "${studio_check}" == true ]]; then
  command -v pnpm >/dev/null 2>&1 || {
    echo "pnpm is required for the atomic Studio v2 release check." >&2
    exit 1
  }
  echo "Validating the matching Orion Studio v2 frontend..."
  pnpm --dir "${project_checkout}/orion_studio" test
  pnpm --dir "${project_checkout}/orion_studio" build
fi
echo "Connecting to ${pi_host} to deploy Orion branch ${branch}..."
# Keep stdin available for sudo. Feeding the script to bash -s prevents a
# normal interactive SSH terminal, so copy it to a temporary file first.
# A private control socket lets setup, transfer and the interactive command reuse
# one login. Close it when deployment ends, including failed transfers.
control_directory="$(mktemp -d /tmp/orion-ssh.XXXXXXXXXX)"
ssh_options=(-o ConnectTimeout=10 -o ControlMaster=auto -o ControlPersist=60
  -o "ControlPath=${control_directory}/connection")
remote_script=""
cleanup_connection() {
  local result=$?
  trap - EXIT
  if [[ -n "${remote_script}" ]]; then
    ssh "${ssh_options[@]}" -o BatchMode=yes "${pi_host}" \
      rm -f -- "${remote_script}" >/dev/null 2>&1 || true
  fi
  ssh "${ssh_options[@]}" -o BatchMode=yes -O exit "${pi_host}" >/dev/null 2>&1 || true
  rm -rf -- "${control_directory}"
  exit "${result}"
}
trap cleanup_connection EXIT
remote_script="$(ssh "${ssh_options[@]}" "${pi_host}" mktemp /tmp/orion-deploy.XXXXXXXXXX)"
if [[ ! "${remote_script}" =~ ^/tmp/orion-deploy\.[A-Za-z0-9]+$ ]]; then
  remote_script=""
  echo "The Pi did not return a valid temporary deployment path." >&2
  exit 1
fi
scp "${ssh_options[@]}" "${script_directory}/pi_deploy_remote.sh" "${pi_host}:${remote_script}"
ssh "${ssh_options[@]}" -t "${pi_host}" "trap 'rm -f -- ${remote_script}' EXIT
bash '${remote_script}' '${project_root}' '${branch}'"
remote_script=""
