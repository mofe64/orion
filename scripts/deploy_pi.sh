#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/deploy_pi.sh [--host USER@HOST] [--root PATH] [--branch BRANCH]

Deploy and physically smoke-test Orion through SSH. Defaults:
  host:   mofe@orion.local
  root:   /home/mofe/dev/orion
  branch: main

The Pi must already trust the workstation's SSH key, and the workstation must
trust the Pi's SSH host key. The remote script installs and enables the oriond
and Orion Studio gateway systemd services as part of the supervised physical
smoke test. The Pi user must have passwordless sudo for service installation
and control.
EOF
}

pi_host="${ORION_PI_HOST:-mofe@orion.local}"
project_root="${ORION_PI_ROOT:-/home/mofe/dev/orion}"
branch="${ORION_PI_BRANCH:-main}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --host) pi_host="${2:?--host requires USER@HOST}"; shift 2 ;;
    --root) project_root="${2:?--root requires PATH}"; shift 2 ;;
    --branch) branch="${2:?--branch requires BRANCH}"; shift 2 ;;
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
echo "Connecting to ${pi_host} to deploy Orion branch ${branch}..."
ssh -o ConnectTimeout=10 "${pi_host}" bash -s -- "${project_root}" "${branch}" \
  < "${script_directory}/pi_deploy_remote.sh"
