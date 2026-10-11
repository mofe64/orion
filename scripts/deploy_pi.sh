#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/deploy_pi.sh --hardware v1|v2 [--host USER@HOST] [--root PATH] [--branch BRANCH] [--prepare-only] [--skip-studio-check]

Probe the robot, build/test a committed release locally in ARM64 Linux,
copy the prepared bundle, and install/activate it without compiling on the Pi.
Defaults: mofe@ariadne-robot.local, /home/mofe/orion, main.

The robot must run Ubuntu 24.04 ARM64 with system Python 3.12, uv and an
installed Python 3.11. Docker must be running locally. The builder and Studio
checks use the exact pushed Git commit. Commit and push the changes first.
--prepare-only installs a release without switching services or moving hardware.
Activation preserves settings, calibration and user assets, and keeps one rollback.
Keep Orion's surroundings clear: activation runs movement, light and audio checks.
--skip-studio-check requires prior validation of this exact commit.
EOF
}

pi_host="${ORION_PI_HOST:-mofe@ariadne-robot.local}"
project_root="${ORION_PI_ROOT:-/home/mofe/orion}"
branch="${ORION_PI_BRANCH:-main}"
hardware="${ORION_PI_HARDWARE:-}"
studio_check=true
prepare_only=false
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
  echo 'Select the target hardware with --hardware v1 or --hardware v2.' >&2; exit 2
fi
if [[ ! "${project_root}" =~ ^/[A-Za-z0-9._/-]+$ || "${project_root}" == *".."* \
      || ! "${branch}" =~ ^[A-Za-z0-9._/-]+$ || "${branch}" == -* || "${branch}" == *".."* \
      || ! "${pi_host}" =~ ^[A-Za-z0-9._-]+@[A-Za-z0-9._-]+$ ]]; then
  echo 'Refusing unsafe deployment arguments' >&2; exit 2
fi
for tool in git python3 docker ssh scp; do
  command -v "${tool}" >/dev/null || { echo "${tool} is required" >&2; exit 1; }
done
script_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_checkout="$(cd "${script_directory}/.." && pwd)"
local_workspace="$(mktemp -d /tmp/orion-local.XXXXXXXXXX)"
control_directory="${local_workspace}/ssh"
mkdir "${control_directory}"
ssh_options=(-o ConnectTimeout=10 -o ControlMaster=auto -o ControlPersist=60
             -o "ControlPath=${control_directory}/connection")
remote_directory=""
cleanup() {
  local result=$?
  trap - EXIT
  if [[ -n "${remote_directory}" ]]; then
    ssh "${ssh_options[@]}" -o BatchMode=yes "${pi_host}" \
      rm -rf -- "${remote_directory}" >/dev/null 2>&1 || true
  fi
  ssh "${ssh_options[@]}" -o BatchMode=yes -O exit "${pi_host}" >/dev/null 2>&1 || true
  rm -rf -- "${local_workspace}"
  exit "${result}"
}
trap cleanup EXIT
git -C "${project_checkout}" fetch origin "+refs/heads/${branch}:refs/remotes/origin/${branch}"
revision="$(git -C "${project_checkout}" rev-parse --verify "origin/${branch}^{commit}")"
if [[ "$(git -C "${project_checkout}" rev-parse HEAD)" != "${revision}" ]]; then
  echo "Warning: local HEAD differs from origin/${branch}; deployment uses origin/${branch}." >&2
fi
if [[ -n "$(git -C "${project_checkout}" status --porcelain)" ]]; then
  echo 'Warning: uncommitted local changes are not deployed.' >&2
fi
release="$(python3 "${script_directory}/build_pi_release.py" --snapshot-only \
  --source "${project_checkout}" --revision "${revision}" --hardware "${hardware}" \
  --output "${local_workspace}/sources")"
if [[ ! -f "${release}/scripts/pi_release_platform.py" ]]; then
  echo 'Commit and push the local-build pipeline before deploying it.' >&2; exit 1
fi
probe_code="$(python3 -c 'import pathlib, shlex, sys; print(shlex.quote(pathlib.Path(sys.argv[1]).read_text()))' \
  "${release}/scripts/pi_release_platform.py")"
echo "Checking ${pi_host}'s OS, architecture and Python before building..."
ssh "${ssh_options[@]}" "${pi_host}" "/usr/bin/python3 -c ${probe_code} --probe" > "${local_workspace}/target.json"
python3 "${release}/scripts/pi_release_platform.py" --target "${local_workspace}/target.json"
build_args=(--prepared-source "${release}" --target "${local_workspace}/target.json" --output "${local_workspace}/release.tar.gz")
if [[ "${studio_check}" == false ]]; then build_args+=(--skip-studio-check); fi
python3 "${release}/scripts/build_pi_release.py" "${build_args[@]}"
digest="$(python3 -c 'import hashlib, sys; f=open(sys.argv[1], "rb"); print(hashlib.file_digest(f, "sha256").hexdigest())' \
  "${local_workspace}/release.tar.gz")"
remote_directory="$(ssh "${ssh_options[@]}" "${pi_host}" mktemp -d /tmp/orion-deploy.XXXXXXXXXX)"
if [[ ! "${remote_directory}" =~ ^/tmp/orion-deploy\.[A-Za-z0-9]+$ ]]; then
  remote_directory=""; echo 'Invalid remote staging directory' >&2; exit 1
fi
echo "Copying the prepared release to ${pi_host}..."
scp "${ssh_options[@]}" "${release}/scripts/pi_deploy_remote.sh" "${pi_host}:${remote_directory}/install.sh"
scp "${ssh_options[@]}" "${local_workspace}/release.tar.gz" "${pi_host}:${remote_directory}/release.tar.gz"
# Keep stdin available for sudo; the script and bundle have already been uploaded.
ssh "${ssh_options[@]}" -t "${pi_host}" "trap 'rm -rf -- ${remote_directory}' EXIT
bash '${remote_directory}/install.sh' '${project_root}' '${remote_directory}/release.tar.gz' '${digest}' '${prepare_only}' '${hardware}'"
