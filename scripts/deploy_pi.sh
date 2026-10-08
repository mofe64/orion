#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/deploy_pi.sh --hardware v1|v2 [--host USER@HOST] [--root PATH] [--branch BRANCH] [--prepare-only] [--skip-studio-check]

Build, test, activate and physically smoke-test the complete Orion Pi stack through SSH. Defaults:
  host:   mofe@ariadne-robot.local
  root:   /home/mofe/orion
  branch: main

Commit and push the intended revision first. The Pi fetches it into an isolated
release without merging, stashing or resetting the checkout. Activation updates
repository-owned pose, motion and scene YAML while preserving user assets and calibration.
Runtime, gateway, Rustpotter/Silero listener, Qwen/Piper workers and the agent
service are prepared before anything is stopped. --prepare-only prints the
prepared release path without switching services. Activation preserves settings
and confirms mechanical rest before switching the hardware runtime. It runs the
light/audio smoke check and both expressive acknowledgement arcs, then confirms
mechanical rest with lights and torque off. Keep Orion's surroundings clear.
Failure restores the immediately previous installation and built-in YAML.
An SSH terminal remains available for sudo.

The matching Studio v2 frontend is tested and production-built locally before
SSH deployment. Use --skip-studio-check only if the exact revision was already
validated on another supported build host.
EOF
}

pi_host="${ORION_PI_HOST:-mofe@ariadne-robot.local}"
project_root="${ORION_PI_ROOT:-/home/mofe/orion}"
branch="${ORION_PI_BRANCH:-main}"
studio_check=true
prepare_only=false
hardware="${ORION_PI_HARDWARE:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --hardware) hardware="${2:?--hardware requires v1 or v2}"; shift 2 ;;
    --host) pi_host="${2:?--host requires USER@HOST}"; shift 2 ;;
    --root) project_root="${2:?--root requires PATH}"; shift 2 ;;
    --branch) branch="${2:?--branch requires BRANCH}"; shift 2 ;;
    --prepare-only) prepare_only=true; shift ;;
    --skip-studio-check) studio_check=false; shift ;;
    --help|-h) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

if [[ "${hardware}" != v1 && "${hardware}" != v2 ]]; then
  echo "Select the target hardware with --hardware v1 or --hardware v2." >&2
  exit 2
fi

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
# The Pi deploys the pushed branch, not this checkout. Warn when they differ.
if git -C "${project_checkout}" fetch -q origin "${branch}" 2>/dev/null; then
  if [[ "$(git -C "${project_checkout}" rev-parse HEAD)" != "$(git -C "${project_checkout}" rev-parse "origin/${branch}")" ]]; then
    echo "Warning: local HEAD differs from origin/${branch}; the Pi deploys origin/${branch}." >&2
  fi
fi
if [[ -n "$(git -C "${project_checkout}" status --porcelain)" ]]; then
  echo "Warning: uncommitted local changes are not deployed." >&2
fi
if [[ "${studio_check}" == true ]]; then
  command -v pnpm >/dev/null 2>&1 || {
    echo "pnpm is required for the atomic Studio v2 release check." >&2
    exit 1
  }
  echo "Validating the matching Orion Studio v2 frontend..."
  pnpm --dir "${project_checkout}/orion_studio" test
  pnpm --dir "${project_checkout}/orion_studio" build
fi
echo "Connecting to ${pi_host} to deploy Orion branch ${branch} for ${hardware}..."
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
bash '${remote_script}' '${project_root}' '${branch}' '${prepare_only}' '${hardware}'"
remote_script=""
